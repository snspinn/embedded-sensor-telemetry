mod usb {
    use std::time::Duration;

    use protocol::TelemetryFrame;
    use rusb::{Device, Direction, GlobalContext, TransferType};

    // Embassy code is: `embassy_usb::Config::new(0x1209, 0x0001);`
    const VID: u16 = 0x1209;
    const PID: u16 = 0x0001;
    const CDC_DATA_CLASS: u8 = 0x0a;

    pub fn read_frames(mut on_frame: impl FnMut(TelemetryFrame)) -> rusb::Result<()> {
        let device = find_device()?;
        let (interface, endpoint) = bulk_in_endpoint(&device)?;

        let handle = device.open()?;
        handle.set_auto_detach_kernel_driver(true)?;
        handle.claim_interface(interface)?;

        let mut buf = [0u8; 64];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let n = match handle.read_bulk(endpoint, &mut buf, Duration::from_secs(1)) {
                Ok(n) => n,
                Err(rusb::Error::Timeout) => continue,
                Err(e) => return Err(e),
            };
            pending.extend_from_slice(&buf[..n]);

            while let Some(end) = pending.iter().position(|&b| b == 0) {
                let mut frame: Vec<u8> = pending.drain(..=end).collect();
                match TelemetryFrame::decode(&mut frame) {
                    Ok(frame) => on_frame(frame),
                    Err(e) => eprintln!("Dropping frame: {e}"),
                }
            }
        }
    }

    fn find_device() -> rusb::Result<Device<GlobalContext>> {
        for device in rusb::devices()?.iter() {
            let desc = device.device_descriptor()?;
            if desc.vendor_id() == VID && desc.product_id() == PID {
                return Ok(device);
            }
        }
        Err(rusb::Error::NoDevice)
    }

    fn bulk_in_endpoint(device: &Device<GlobalContext>) -> rusb::Result<(u8, u8)> {
        let config = device.active_config_descriptor()?;
        for interface in config.interfaces() {
            for alt in interface.descriptors() {
                if alt.class_code() != CDC_DATA_CLASS {
                    continue;
                }
                for ep in alt.endpoint_descriptors() {
                    if ep.direction() == Direction::In && ep.transfer_type() == TransferType::Bulk {
                        return Ok((alt.interface_number(), ep.address()));
                    }
                }
            }
        }
        Err(rusb::Error::NotFound)
    }
}

#[tokio::main]
async fn main() {
    let result = tokio::task::spawn_blocking(|| usb::read_frames(|frame| println!("{frame:?}")))
        .await
        .expect("USB reader panicked");

    if let Err(e) = result {
        eprintln!("USB error: {e}");
    }
}

#[cfg(test)]
mod tests {
    use protocol::*;

    #[test]
    fn roundtrip() {
        let frame = TelemetryFrame::new(
            42,
            12345678,
            ImuFusion {
                roll: 0.1,
                pitch: 0.01,
                yaw: 0.5,
            },
        );

        let encoded = &mut frame.encode().expect("encode");
        let bytes: &mut [u8] = &mut encoded[..];
        let decoded = TelemetryFrame::decode(bytes).expect("decode");

        assert_eq!(frame, decoded);
    }
}
