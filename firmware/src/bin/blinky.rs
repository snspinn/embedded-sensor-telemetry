#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::Timer;
use panic_probe as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    info!("Hello World!");

    let mut led_3 = Output::new(p.PE9, Level::Low, Speed::Low); // N, red
    let mut led_4 = Output::new(p.PE8, Level::Low, Speed::Low); // NW, blue
    let mut led_5 = Output::new(p.PE10, Level::Low, Speed::Low); // NE, orange
    let mut led_6 = Output::new(p.PE15, Level::Low, Speed::Low); // W, green
    let mut led_7 = Output::new(p.PE11, Level::Low, Speed::Low); // E, green
    let mut led_8 = Output::new(p.PE14, Level::Low, Speed::Low); // SW, orange
    let mut led_9 = Output::new(p.PE12, Level::Low, Speed::Low); // SE, blue
    let mut led_10 = Output::new(p.PE13, Level::Low, Speed::Low); // S, red

    loop {
        led_3.toggle();
        Timer::after_millis(100).await;
        led_5.toggle();
        Timer::after_millis(100).await;
        led_7.toggle();
        Timer::after_millis(100).await;
        led_9.toggle();
        Timer::after_millis(100).await;
        led_10.toggle();
        Timer::after_millis(100).await;
        led_8.toggle();
        Timer::after_millis(100).await;
        led_6.toggle();
        Timer::after_millis(100).await;
        led_4.toggle();
        Timer::after_millis(100).await;
        info!("loop");
    }
}

