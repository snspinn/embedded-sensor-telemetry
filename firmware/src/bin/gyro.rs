#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts, dma,
    gpio::{Level, Output, Speed},
    interrupt, peripherals,
    spi::{self, BitOrder, Config as SpiConfig, MODE_3, Spi},
    time::Hertz,
};
use embassy_time::Timer;

// L3GD20 registers
const WHO_AM_I: u8 = 0x0F;
const CTRL_REG1: u8 = 0x20;
const OUT_X_L: u8 = 0x28;
const READ_FLAG: u8 = 0x80; // bit 7 = read
const AUTO_INC: u8 = 0x40; // bit 6 = auto-increment address

// Bind the DMA channel interrupts — NOT spi::InterruptHandler<SPI1>
bind_interrupts!(struct Irqs {
    DMA1_CHANNEL3 => dma::InterruptHandler<peripherals::DMA1_CH3>; // TX
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>; // RX
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    // PE3 = CS, active low
    let mut cs = Output::new(p.PE3, Level::High, Speed::VeryHigh);

    let mut config = SpiConfig::default();
    config.mode = MODE_3; // L3GD20 requires CPOL=1, CPHA=1
    config.bit_order = BitOrder::MsbFirst;
    config.frequency = Hertz(1_000_000);

    // PA5=SCK, PA6=MISO, PA7=MOSI (hardwired on F3 Discovery)
    let mut spi = Spi::new(
        p.SPI1, p.PA5,      // SCK
        p.PA7,      // MOSI
        p.PA6,      // MISO
        p.DMA1_CH3, // TX DMA
        p.DMA1_CH2, // RX DMA
        Irqs, config,
    );

    // Enable gyro: normal mode, all axes on (0x0F)
    cs.set_low();
    spi.write(&[CTRL_REG1, 0x0F]).await.unwrap();
    cs.set_high();

    let mut buf = [0u8; 7]; // 1 cmd byte + 6 data bytes

    loop {
        // Read 6 bytes starting at OUT_X_L with auto-increment
        let cmd = READ_FLAG | AUTO_INC | OUT_X_L;
        let tx = [cmd, 0, 0, 0, 0, 0, 0];

        cs.set_low();
        spi.transfer(&mut buf, &tx).await.unwrap();
        cs.set_high();

        // buf[0] is the dummy byte clocked out during cmd phase
        let x = i16::from_le_bytes([buf[1], buf[2]]);
        let y = i16::from_le_bytes([buf[3], buf[4]]);
        let z = i16::from_le_bytes([buf[5], buf[6]]);

        // At 250 dps range: 1 LSB ≈ 8.75 mdps
        info!("Gyro  x={} raw  y={} raw  z={} raw", x, y, z);

        Timer::after_millis(100).await;
    }
}
