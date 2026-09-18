#![no_std]
#![no_main]

use ahrs::{Ahrs, Madgwick};
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::Peri;
use embassy_stm32::Peripherals;
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::i2c::mode::Master as I2cMaster;
use embassy_stm32::mode::Async;
use embassy_stm32::spi::mode::Master as SPIMaster;
use embassy_stm32::{
    bind_interrupts, dma,
    gpio::{Level, Output, Speed},
    i2c::{self, I2c},
    interrupt, peripherals,
    spi::{self, BitOrder, Config as SpiConfig, MODE_3, Spi},
    time::Hertz,
};
use embassy_time::Timer;
use nalgebra::Vector3;
use panic_probe as _;

// LSM303DLHC accelerometer I2C address and registers
const ACCEL_ADDR: u8 = 0x19;
const CTRL_REG1_A: u8 = 0x20; // enable all axes, 100 Hz ODR
const OUT_X_L_A: u8 = 0x28 | 0x80; // 0x80 = auto-increment bit
const MAG_ADDR: u8 = 0x1E;
const CRA_REG_M: u8 = 0x00;
const MR_REG_M: u8 = 0x02;
const OUT_X_H_M: u8 = 0x03;

bind_interrupts!(struct AccelInterrupts {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    DMA1_CHANNEL6 => dma::InterruptHandler<peripherals::DMA1_CH6>; // TX
    DMA1_CHANNEL7 => dma::InterruptHandler<peripherals::DMA1_CH7>; // RX
});

// L3GD20 registers
const CTRL_REG1: u8 = 0x20;
const OUT_X_L: u8 = 0x28;
const READ_FLAG: u8 = 0x80; // bit 7 = read
const AUTO_INC: u8 = 0x40; // bit 6 = auto-increment address

bind_interrupts!(struct GyroInterrupts {
    DMA1_CHANNEL3 => dma::InterruptHandler<peripherals::DMA1_CH3>; // TX
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>; // RX
});

#[derive(defmt::Format)]
struct Vec3 {
    x: i16,
    y: i16,
    z: i16,
}

#[derive(defmt::Format)]
struct CombinedImu {
    accel: Vec3,
    gyro: Vec3,
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut p = embassy_stm32::init(Default::default());

    let heartbeat_led = Output::new(p.PE9, Level::Low, Speed::Low); // N, red
    spawner.spawn(heartbeat(heartbeat_led).unwrap());

    /* Set up the gyroscope */
    // PE3 = CS, active low
    let mut gyro_cs = Output::new(p.PE3, Level::High, Speed::VeryHigh);
    let mut gyro_config = SpiConfig::default();
    gyro_config.mode = MODE_3; // L3GD20 requires CPOL=1, CPHA=1
    gyro_config.bit_order = BitOrder::MsbFirst;
    gyro_config.frequency = Hertz(1_000_000);

    // PA5=SCK, PA6=MISO, PA7=MOSI (hardwired on F3 Discovery)
    let mut gyro_spi = Spi::new(
        p.SPI1,
        p.PA5,      // SCK
        p.PA7,      // MOSI
        p.PA6,      // MISO
        p.DMA1_CH3, // TX DMA
        p.DMA1_CH2, // RX DMA
        GyroInterrupts,
        gyro_config,
    );
    // Enable gyro: normal mode, all axes on (0x0F)
    gyro_cs.set_low();
    gyro_spi.write(&[CTRL_REG1, 0x0F]).await.unwrap();
    gyro_cs.set_high();
    let mut gyro_buf = [0u8; 7]; // 1 cmd byte + 6 data bytes

    /* Set up the accelerometer & magnetometer */
    let mut config = I2cConfig::default();
    config.frequency = Hertz(400_000);
    // PB6 = SCL, PB7 = SDA (hardwired on F3 Discovery)
    let mut i2c = configure_accel(
        config,
        p.I2C1.reborrow(),
        p.PB6.reborrow(),
        p.PB7.reborrow(),
        p.DMA1_CH6.reborrow(),
        p.DMA1_CH7.reborrow(),
    )
    .await;
    let mut accel_buf = [0u8; 6];
    /* Set up the magnetometer */
    // Configure magnetometer: 15 Hz ODR, continuous mode
    i2c.write(MAG_ADDR, &[CRA_REG_M, 0x10]).await.unwrap();
    i2c.write(MAG_ADDR, &[MR_REG_M, 0x00]).await.unwrap();
    let mut mag_buf = [0u8; 6];

    loop {
        let ((accel, mag), gyro) = join(
            read_i2c_sensors(&mut i2c, (&mut accel_buf, &mut mag_buf)),
            read_gyro(&mut gyro_spi, &mut gyro_cs, &mut gyro_buf),
        )
        .await;

        let imu = CombinedImu { accel, gyro };
        info!("IMU {}", imu);
        Timer::after_millis(100).await;
    }
}

async fn read_i2c_sensors(
    i2c: &mut I2c<'_, Async, I2cMaster>,
    buf: (&mut [u8], &mut [u8]),
) -> (Vec3, Vec3) {
    // Write register address, then read 6 bytes (X_L, X_H, Y_L, Y_H, Z_L, Z_H)
    i2c.write_read(ACCEL_ADDR, &[OUT_X_L_A], buf.0)
        .await
        .unwrap();

    let a_x = i16::from_le_bytes([buf.0[0], buf.0[1]]) >> 4; // 12-bit left-justified
    let a_y = i16::from_le_bytes([buf.0[2], buf.0[3]]) >> 4;
    let a_z = i16::from_le_bytes([buf.0[4], buf.0[5]]) >> 4;
    // At ±2g range: 1 LSB = 1 mg
    debug!("Accel  x={} mg  y={} mg  z={} mg", a_x, a_y, a_z);

    i2c.write_read(MAG_ADDR, &[OUT_X_H_M], buf.1).await.unwrap();
    // LSM303DLHC byte order: X_H, X_L, Z_H, Z_L, Y_H, Y_L
    let m_x = i16::from_be_bytes([buf.1[0], buf.1[1]]);
    let m_z = i16::from_be_bytes([buf.1[2], buf.1[3]]);
    let m_y = i16::from_be_bytes([buf.1[4], buf.1[5]]);
    debug!("Mag raw  X={}  Y={}  Z={}", m_x, m_y, m_z);

    (
        Vec3 {
            x: a_x,
            y: a_y,
            z: a_z,
        },
        Vec3 {
            x: m_x,
            y: m_y,
            z: m_z,
        },
    )
}

async fn read_gyro<'a>(
    spi: &mut Spi<'_, Async, SPIMaster>,
    cs: &mut Output<'a>,
    buf: &mut [u8],
) -> Vec3 {
    // Read 6 bytes starting at OUT_X_L with auto-increment
    let cmd = READ_FLAG | AUTO_INC | OUT_X_L;
    let tx = [cmd, 0, 0, 0, 0, 0, 0];

    cs.set_low();
    spi.transfer(buf, &tx).await.unwrap();
    cs.set_high();

    // buf[0] is the dummy byte clocked out during cmd phase
    let x = i16::from_le_bytes([buf[1], buf[2]]);
    let y = i16::from_le_bytes([buf[3], buf[4]]);
    let z = i16::from_le_bytes([buf[5], buf[6]]);

    // At 250 dps range: 1 LSB ≈ 8.75 mdps
    Vec3 { x, y, z }
}

/// Configure accelerometer peripherals. Calling this will require using `reborrow()`.
///
/// Usage example:
/// ```rust
/// let mut p = embassy_stm32::init(Default::default());
///
/// let i2c = configure_accel(
///    p.I2C1.reborrow(),
///    p.PB6.reborrow(),
///    p.PB7.reborrow(),
///    p.DMA1_CH6.reborrow(),
///    p.DMA1_CH7.reborrow(),
/// ).await;
/// ```
async fn configure_accel<'d>(
    config: i2c::Config,
    i2c: Peri<'d, peripherals::I2C1>,
    scl: Peri<'d, peripherals::PB6>,
    sda: Peri<'d, peripherals::PB7>,
    tx_dma: Peri<'d, peripherals::DMA1_CH6>,
    rx_dma: Peri<'d, peripherals::DMA1_CH7>,
) -> I2c<'d, Async, I2cMaster> {
    let mut accel_i2c = I2c::new(i2c, scl, sda, tx_dma, rx_dma, AccelInterrupts, config);
    // Enable accelerometer: 100 Hz, all axes on (0x57)
    accel_i2c
        .write(ACCEL_ADDR, &[CTRL_REG1_A, 0x57])
        .await
        .unwrap();
    accel_i2c
}

#[embassy_executor::task]
async fn heartbeat(mut led: Output<'static>) {
    // 10 second heartbeat
    loop {
        led.set_high();
        Timer::after_millis(100).await;
        led.set_low();
        Timer::after_millis(9900).await;
    }
}
