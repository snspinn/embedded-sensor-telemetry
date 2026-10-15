#![no_std]
#![no_main]

use defmt_rtt as _;
use defmt_test::tests;
use panic_probe as _;
use protocol::{ImuFusion, TelemetryFrame};

#[tests]
mod tests {
    use super::*;

    #[test]
    fn protocol_roundtrip() {
        let original = TelemetryFrame::new(
            123,
            42_000,
            ImuFusion {
                roll: 12.5,
                pitch: -3.2,
                yaw: 45.0,
            },
        );

        let encoded = &mut original.encode().expect("encode");
        let bytes: &mut [u8] = &mut encoded[..];
        let decoded = TelemetryFrame::decode(bytes).unwrap();

        defmt::assert_eq!(original.version, decoded.version);
        defmt::assert_eq!(original.seq.0, decoded.seq.0);
        defmt::assert_eq!(original.uptime_ms, decoded.uptime_ms);
        defmt::assert_eq!(original.imu.roll, decoded.imu.roll);
        defmt::assert_eq!(original.imu.pitch, decoded.imu.pitch);
        defmt::assert_eq!(original.imu.yaw, decoded.imu.yaw);

        defmt::info!("protocol roundtrip test passed");
    }
}
