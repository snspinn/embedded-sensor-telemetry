#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::i2c::mode::Master;
use embassy_stm32::mode::Async;
use embassy_stm32::{
    bind_interrupts, dma,
    gpio::{Level, Output, Speed},
    i2c::{self, I2c},
    interrupt, peripherals,
    spi::{self, BitOrder, Config as SpiConfig, MODE_3, Spi},
    time::Hertz,
};
use embassy_time::Timer;
use panic_probe as _;

// LSM303DLHC accelerometer I2C address and registers
const ACCEL_ADDR: u8 = 0x19;
const CTRL_REG1_A: u8 = 0x20; // enable all axes, 100 Hz ODR
const OUT_X_L_A: u8 = 0x28 | 0x80; // 0x80 = auto-increment bit

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

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

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

    /* Set up the accelerometer */
    let mut accel_config = I2cConfig::default();
    accel_config.frequency = Hertz(400_000);
    // PB6 = SCL, PB7 = SDA (hardwired on F3 Discovery)
    let mut accel_i2c = I2c::new(
        p.I2C1,
        p.PB6,      // SCL
        p.PB7,      // SDA
        p.DMA1_CH6, // TX DMA
        p.DMA1_CH7, // RX DMA
        AccelInterrupts,
        accel_config,
    );
    // Enable accelerometer: 100 Hz, all axes on (0x57)
    accel_i2c
        .write(ACCEL_ADDR, &[CTRL_REG1_A, 0x57])
        .await
        .unwrap();
    let mut accel_buf = [0u8; 6];

    loop {
        /* Read gyro */
        // Read 6 bytes starting at OUT_X_L with auto-increment
        let cmd = READ_FLAG | AUTO_INC | OUT_X_L;
        let tx = [cmd, 0, 0, 0, 0, 0, 0];

        gyro_cs.set_low();
        gyro_spi.transfer(&mut gyro_buf, &tx).await.unwrap();
        gyro_cs.set_high();

        // gyro_buf[0] is the dummy byte clocked out during cmd phase
        let x = i16::from_le_bytes([gyro_buf[1], gyro_buf[2]]);
        let y = i16::from_le_bytes([gyro_buf[3], gyro_buf[4]]);
        let z = i16::from_le_bytes([gyro_buf[5], gyro_buf[6]]);

        // At 250 dps range: 1 LSB ≈ 8.75 mdps
        info!("Gyro  x={} raw  y={} raw  z={} raw", x, y, z);

        Timer::after_millis(100).await;

        let accel = read_accel(&mut accel_i2c, &mut accel_buf);
    }
}

async fn read_accel(i2c: &mut I2c<'_, Async, Master>, buf: &mut [u8]) -> Vec3 {
    // Write register address, then read 6 bytes (X_L, X_H, Y_L, Y_H, Z_L, Z_H)
    i2c.write_read(ACCEL_ADDR, &[OUT_X_L_A], buf).await.unwrap();

    let x = i16::from_le_bytes([buf[0], buf[1]]) >> 4; // 12-bit left-justified
    let y = i16::from_le_bytes([buf[2], buf[3]]) >> 4;
    let z = i16::from_le_bytes([buf[4], buf[5]]) >> 4;

    // At ±2g range: 1 LSB = 1 mg
    info!("Accel  x={} mg  y={} mg  z={} mg", x, y, z);
    Vec3 { x, y, z }
}
