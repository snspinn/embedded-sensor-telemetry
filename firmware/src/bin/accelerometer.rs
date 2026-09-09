#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_stm32::i2c::Config as I2cConfig;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts, dma,
    i2c::{self, I2c},
    peripherals,
    time::Hertz,
};
use embassy_time::Timer;

// LSM303DLHC accelerometer I2C address and registers
const ACCEL_ADDR: u8 = 0x19;
const CTRL_REG1_A: u8 = 0x20; // enable all axes, 100 Hz ODR
const OUT_X_L_A: u8 = 0x28 | 0x80; // 0x80 = auto-increment bit

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    DMA1_CHANNEL6 => dma::InterruptHandler<peripherals::DMA1_CH6>; // TX
    DMA1_CHANNEL7 => dma::InterruptHandler<peripherals::DMA1_CH7>; // RX
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    let mut config = I2cConfig::default();
    config.frequency = Hertz(400_000);
    // PB6 = SCL, PB7 = SDA (hardwired on F3 Discovery)
    let mut i2c = I2c::new(
        p.I2C1, p.PB6,      // SCL
        p.PB7,      // SDA
        p.DMA1_CH6, // TX DMA
        p.DMA1_CH7, // RX DMA
        Irqs, config,
    );

    // Enable accelerometer: 100 Hz, all axes on (0x57)
    i2c.write(ACCEL_ADDR, &[CTRL_REG1_A, 0x57]).await.unwrap();

    let mut buf = [0u8; 6];

    loop {
        // Write register address, then read 6 bytes (X_L, X_H, Y_L, Y_H, Z_L, Z_H)
        i2c.write_read(ACCEL_ADDR, &[OUT_X_L_A], &mut buf)
            .await
            .unwrap();

        let x = i16::from_le_bytes([buf[0], buf[1]]) >> 4; // 12-bit left-justified
        let y = i16::from_le_bytes([buf[2], buf[3]]) >> 4;
        let z = i16::from_le_bytes([buf[4], buf[5]]) >> 4;

        // At ±2g range: 1 LSB = 1 mg
        info!("Accel  x={} mg  y={} mg  z={} mg", x, y, z);

        Timer::after_millis(100).await;
    }
}
