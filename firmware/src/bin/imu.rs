#![no_std]
#![no_main]

use ahrs::{Ahrs, Madgwick};
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::i2c::mode::Master as I2cMaster;
use embassy_stm32::mode::Async;
use embassy_stm32::spi::mode::Master as SPIMaster;
use embassy_stm32::usb::{self, Driver};
use embassy_stm32::{
    bind_interrupts, dma,
    gpio::{Level, Output, Speed},
    i2c::{self, I2c},
    peripherals,
    spi::{BitOrder, Config as SpiConfig, MODE_3, Spi},
    time::Hertz,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Ticker, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, UsbDevice};
use nalgebra::Vector3;
use panic_probe as _;
use protocol::{ImuFusion, Seq, TelemetryFrame};
use static_cell::StaticCell;

// Sampling period
const SAMPLE_PERIOD_MS: u64 = 100;
const SAMPLE_PERIOD_S: f32 = SAMPLE_PERIOD_MS as f32 / 1000.0;

// LSM303AGR accelerometer I2C address and registers
const ACCEL_ADDR: u8 = 0x19;
const CTRL_REG1_A: u8 = 0x20; // enable all axes, 100 Hz ODR
const CTRL_REG4_A: u8 = 0x23;
const OUT_X_L_A: u8 = 0x28 | 0x80; // 0x80 = auto-increment bit
const MAG_ADDR: u8 = 0x1E;
const CFG_REG_A_M: u8 = 0x60;
const CFG_REG_C_M: u8 = 0x62;
const OUTX_L_REG_M: u8 = 0x68;

// Conversion factors
const MAG_SENS: f32 = 0.0015; // gauss per LSB, same on all axes, no gain setting
const ACCEL_SENS: f32 = 0.001; // g/LSB at ±2g (1 mg/LSB)
const G: f32 = 9.80665; // m/s²
const GYRO_SENS: f32 = 8.75e-3; // dps/LSB at ±250 dps
const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;

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

// USB interrupt
bind_interrupts!(struct UsbIrqs {
    USB_LP_CAN_RX0 => usb::InterruptHandler<peripherals::USB>;
});

type UsbDriver = Driver<'static, peripherals::USB>;

struct UsbResources {
    config_desc: [u8; 256],
    bos_desc: [u8; 256],
    control_buf: [u8; 64],
    cdc_state: State<'static>,
}

static USB_RES: StaticCell<UsbResources> = StaticCell::new();

// 8 frames at 10 Hz allows for 800 ms of hiccups on USB side
static FRAMES: Channel<CriticalSectionRawMutex, TelemetryFrame, 8> = Channel::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    // -- USB settings for RCC output (data streaming) --
    {
        use embassy_stm32::rcc::*;
        use embassy_stm32::time::mhz;
        config.rcc.hse = Some(Hse {
            freq: mhz(8),
            mode: HseMode::Bypass,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL9,
        }); // 72 MHz
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2; // APB1 is limited to 36 MHz
        config.rcc.apb2_pre = APBPrescaler::DIV1;
    }
    // Embassy init
    let mut p = embassy_stm32::init(config);
    // The Discovery board has a fixed pull-up resistor on D+ (PA12). Holding PA12
    // low for a moment looks like an unplug to the host, so it re-enumerates after every reflash.
    {
        let _dp = Output::new(p.PA12.reborrow(), Level::Low, Speed::Low);
        Timer::after_millis(10).await;
    }
    let driver = Driver::new(p.USB, UsbIrqs, p.PA12, p.PA11);

    let mut usb_config = embassy_usb::Config::new(0x1209, 0x0001); // Test ID
    usb_config.manufacturer = Some("Samuel Spinn");
    usb_config.product = Some("IMU telemetery");
    usb_config.serial_number = Some("0001");

    let res = USB_RES.init(UsbResources {
        config_desc: [0; 256],
        bos_desc: [0; 256],
        control_buf: [0; 64],
        cdc_state: State::new(),
    });

    let mut builder = Builder::new(
        driver,
        usb_config,
        &mut res.config_desc,
        &mut res.bos_desc,
        &mut [],
        &mut res.control_buf,
    );
    let mut class = CdcAcmClass::new(&mut builder, &mut res.cdc_state, 64);
    let usb = builder.build();
    spawner.spawn(usb_task(usb).unwrap());

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
    let gyro_buf = [0u8; 7]; // 1 cmd byte + 6 data bytes

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

    // Print out chip details
    let mut id = [0u8; 1];
    match i2c.write_read(MAG_ADDR, &[0x4F], &mut id).await {
        Ok(()) => info!("WHO_AM_I_M = {:#04x} (0x40 means LSM303AGR)", id[0]),
        Err(e) => warn!("WHO_AM_I_M read failed: {:?}", e),
    }

    // -- Accelerometer init --  100 Hz, all axes on (0x57)
    i2c.write(ACCEL_ADDR, &[CTRL_REG1_A, 0x57]).await.unwrap(); // 100 HZ, all axes on
    i2c.write(ACCEL_ADDR, &[CTRL_REG4_A, 0x08]).await.unwrap(); // ±2g full-scale, high-res mode (HR bit)

    let accel_buf = [0u8; 6];
    // -- Magnetometer init --
    // Temperature compensation on, 10 Hz, continuous mode
    i2c.write(MAG_ADDR, &[CFG_REG_A_M, 0x80]).await.unwrap();
    // Block data update, so the high and low bytes always come from the same sample
    i2c.write(MAG_ADDR, &[CFG_REG_C_M, 0x10]).await.unwrap();
    let mag_buf = [0u8; 6];

    spawner.spawn(sensor_task(i2c, gyro_spi, gyro_cs, accel_buf, mag_buf, gyro_buf).unwrap());
    spawner.spawn(telemetry_task(class).unwrap());
}

async fn read_i2c_sensors(
    i2c: &mut I2c<'_, Async, I2cMaster>,
    buf: (&mut [u8], &mut [u8]),
) -> (Vector3<f32>, Vector3<f32>) {
    i2c.write_read(ACCEL_ADDR, &[OUT_X_L_A], buf.0)
        .await
        .unwrap();

    let accel = Vector3::new(
        (i16::from_le_bytes([buf.0[0], buf.0[1]]) >> 4) as f32 * ACCEL_SENS * G,
        (i16::from_le_bytes([buf.0[2], buf.0[3]]) >> 4) as f32 * ACCEL_SENS * G,
        (i16::from_le_bytes([buf.0[4], buf.0[5]]) >> 4) as f32 * ACCEL_SENS * G,
    );

    i2c.write_read(MAG_ADDR, &[OUTX_L_REG_M], buf.1)
        .await
        .unwrap();
    let mag = Vector3::new(
        i16::from_le_bytes([buf.1[0], buf.1[1]]) as f32 * MAG_SENS,
        i16::from_le_bytes([buf.1[2], buf.1[3]]) as f32 * MAG_SENS,
        i16::from_le_bytes([buf.1[4], buf.1[5]]) as f32 * MAG_SENS,
    );

    (accel, mag)
}

async fn read_gyro<'a>(
    spi: &mut Spi<'_, Async, SPIMaster>,
    cs: &mut Output<'a>,
    buf: &mut [u8],
) -> Vector3<f32> {
    // Read 6 bytes starting at OUT_X_L with auto-increment
    let cmd = READ_FLAG | AUTO_INC | OUT_X_L;
    let tx = [cmd, 0, 0, 0, 0, 0, 0];

    cs.set_low();
    spi.transfer(buf, &tx).await.unwrap();
    cs.set_high();

    // gyro_buf[0] is the command echo; data starts at [1]
    Vector3::new(
        i16::from_le_bytes([buf[1], buf[2]]) as f32 * GYRO_SENS * DEG_TO_RAD,
        i16::from_le_bytes([buf[3], buf[4]]) as f32 * GYRO_SENS * DEG_TO_RAD,
        i16::from_le_bytes([buf[5], buf[6]]) as f32 * GYRO_SENS * DEG_TO_RAD,
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

#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, UsbDriver>) -> ! {
    usb.run().await
}

async fn write_frame(
    class: &mut CdcAcmClass<'static, UsbDriver>,
    data: heapless::Vec<u8, 64>,
) -> Result<(), EndpointError> {
    let max = class.max_packet_size() as usize;
    for chunk in data.chunks(max) {
        class.write_packet(chunk).await?;
    }
    if data.len() % max == 0 {
        class.write_packet(&[]).await?;
    }
    Ok(())
}

#[embassy_executor::task]
async fn sensor_task(
    mut i2c: I2c<'static, Async, I2cMaster>,
    mut spi: Spi<'static, Async, SPIMaster>,
    mut gyro_cs: Output<'static>,
    mut accel_buf: [u8; 6],
    mut mag_buf: [u8; 6],
    mut gyro_buf: [u8; 7],
) -> ! {
    // do thing
    let mut ahrs = Madgwick::new(SAMPLE_PERIOD_S, 0.1f32);
    let mut ticker = Ticker::every(Duration::from_millis(SAMPLE_PERIOD_MS));
    let mut sequence: u32 = 0;

    loop {
        ticker.next().await;
        let ((accel, mag), gyro) = join(
            read_i2c_sensors(&mut i2c, (&mut accel_buf, &mut mag_buf)),
            read_gyro(&mut spi, &mut gyro_cs, &mut gyro_buf),
        )
        .await;

        if mag.norm() < 1e-3 {
            error!("Degenerate mag vector: {:?}", (mag.x, mag.y, mag.z));
            continue; // don't feed bad data into the filter
        }
        // Run inputs through AHRS filter (gyroscope must be radians/s)
        let quat = match ahrs.update(&gyro, &accel, &mag) {
            Ok(quat) => quat,
            Err(_e) => {
                warn!("AHRS update failed");
                continue;
            }
        };
        let (roll, pitch, yaw) = quat.euler_angles();
        // Do something with the updated state quaternion
        info!("pitch={}, roll={}, yaw={}", pitch, roll, yaw);
        sequence += 1;
        let frame: TelemetryFrame = TelemetryFrame::new(
            sequence,
            Instant::now().as_millis(),
            ImuFusion { roll, pitch, yaw },
        );
        // Note: `try_send()` drops newest frames when channel is full
        // TODO: Keep freshest data with a `Signal` or `Watch`
        let _ = FRAMES.try_send(frame);
    }
}

#[embassy_executor::task]
async fn telemetry_task(mut class: CdcAcmClass<'static, UsbDriver>) -> ! {
    let mut buf = [0u8; 64];
    loop {
        class.wait_connection().await;
        FRAMES.clear(); // drop samples queued up before we started listening
        loop {
            let frame = FRAMES.receive().await;
            let Ok(bytes) = frame.encode() else {
                warn!("COBs encode failed");
                continue;
            };
            if write_frame(&mut class, bytes).await.is_err() {
                break; // disconnected, go back to waiting
            }
        }
    }
}
