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
use nalgebra::Vector;
use nalgebra::Vector3;
use panic_probe as _;

// LSM303DLHC accelerometer I2C address and registers
const ACCEL_ADDR: u8 = 0x19;
const CTRL_REG1_A: u8 = 0x20; // enable all axes, 100 Hz ODR
const CTRL_REG4_A: u8 = 0x23;
const OUT_X_L_A: u8 = 0x28 | 0x80; // 0x80 = auto-increment bit
const MAG_ADDR: u8 = 0x1E;
const CRA_REG_M: u8 = 0x00;
const MR_REG_M: u8 = 0x02;
const OUT_X_H_M: u8 = 0x03;
const CRB_REG_M: u8 = 0x01; // mag gain register

// Conversion factors
const MAG_XY_GAIN: f64 = 1100.0; // LSB/Gauss, GN=001 (default)
const MAG_Z_GAIN: f64 = 980.0; // LSB/Gauss, GN=001 — Z is different!
const ACCEL_SENS: f64 = 0.001; // g/LSB at ±2g (1 mg/LSB)
const G: f64 = 9.80665; // m/s²
const GYRO_SENS: f64 = 8.75e-3; // dps/LSB at ±250 dps
const DEG_TO_RAD: f64 = core::f64::consts::PI / 180.0;

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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

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
    let mut i2c = I2c::new(
        p.I2C1,
        p.PB6,      //scl
        p.PB7,      // sda
        p.DMA1_CH6, // tx dma
        p.DMA1_CH7, // rw dma
        AccelInterrupts,
        config,
    );
    // -- Accelerometer init --  100 Hz, all axes on (0x57)
    i2c.write(ACCEL_ADDR, &[CTRL_REG1_A, 0x57]).await.unwrap(); // 100 HZ, all axes on
    i2c.write(ACCEL_ADDR, &[CTRL_REG4_A, 0x08]).await.unwrap(); // ±2g full-scale, high-res mode (HR bit)

    let mut accel_buf = [0u8; 6];
    // -- Magnetometer init --
    i2c.write(MAG_ADDR, &[CRA_REG_M, 0x10]).await.unwrap(); // 15 Hz ODR
    i2c.write(MAG_ADDR, &[CRB_REG_M, 0x20]).await.unwrap(); // gain GN=001 → ±1.3 Gauss full-scale (most sensitive)
    i2c.write(MAG_ADDR, &[MR_REG_M, 0x00]).await.unwrap(); // continuous conversion
    let mut mag_buf = [0u8; 6];

    let mut ahrs = Madgwick::default();
    loop {
        let ((accel, mag), gyro) = join(
            read_i2c_sensors(&mut i2c, (&mut accel_buf, &mut mag_buf)),
            read_gyro(&mut gyro_spi, &mut gyro_cs, &mut gyro_buf),
        )
        .await;

        // Run inputs through AHRS filter (gyroscope must be radians/s)
        let quat = ahrs.update(&gyro, &accel, &mag).unwrap();
        let (roll, pitch, yaw) = quat.euler_angles();
        // Do something with the updated state quaternion
        println!("pitch={}, roll={}, yaw={}", pitch, roll, yaw);
        Timer::after_millis(100).await;
    }
}

async fn read_i2c_sensors(
    i2c: &mut I2c<'_, Async, I2cMaster>,
    buf: (&mut [u8], &mut [u8]),
) -> (Vector3<f64>, Vector3<f64>) {
    // Write register address, then read 6 bytes (X_L, X_H, Y_L, Y_H, Z_L, Z_H)
    i2c.write_read(ACCEL_ADDR, &[OUT_X_L_A], buf.0)
        .await
        .unwrap();

    // Note  LSM303DLHC accelerometer data is left-aligned
    let accel = Vector3::new(
        (i16::from_le_bytes([buf.0[0], buf.0[1]]) >> 4) as f64 * ACCEL_SENS * G,
        (i16::from_le_bytes([buf.0[2], buf.0[3]]) >> 4) as f64 * ACCEL_SENS * G,
        (i16::from_le_bytes([buf.0[4], buf.0[5]]) >> 4) as f64 * ACCEL_SENS * G,
    );

    i2c.write_read(MAG_ADDR, &[OUT_X_H_M], buf.1).await.unwrap();

    // Note LSM303DLHC magnetometer output byte order (Z before Y):
    //   X_H, X_L, Z_H, Z_L, Y_H, Y_L
    let mag = Vector3::new(
        i16::from_be_bytes([buf.1[0], buf.1[1]]) as f64 / MAG_XY_GAIN,
        i16::from_be_bytes([buf.1[4], buf.1[5]]) as f64 / MAG_XY_GAIN,
        i16::from_be_bytes([buf.1[2], buf.1[3]]) as f64 / MAG_Z_GAIN,
    );

    (accel, mag)
}

async fn read_gyro<'a>(
    spi: &mut Spi<'_, Async, SPIMaster>,
    cs: &mut Output<'a>,
    buf: &mut [u8],
) -> Vector3<f64> {
    // Read 6 bytes starting at OUT_X_L with auto-increment
    let cmd = READ_FLAG | AUTO_INC | OUT_X_L;
    let tx = [cmd, 0, 0, 0, 0, 0, 0];

    cs.set_low();
    spi.transfer(buf, &tx).await.unwrap();
    cs.set_high();

    // gyro_buf[0] is the command echo; data starts at [1]
    Vector3::new(
        i16::from_le_bytes([buf[1], buf[2]]) as f64 * GYRO_SENS * DEG_TO_RAD,
        i16::from_le_bytes([buf[3], buf[4]]) as f64 * GYRO_SENS * DEG_TO_RAD,
        i16::from_le_bytes([buf[5], buf[6]]) as f64 * GYRO_SENS * DEG_TO_RAD,
    )
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
