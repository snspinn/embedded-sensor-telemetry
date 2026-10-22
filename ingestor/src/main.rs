// extern crate hidapi;
use protocol;
use rusb;

mod serial {
    use rusb::DeviceHandle;

    pub async fn connect() {
        for device in rusb::devices().unwrap().iter() {
            let device_desc = device.device_descriptor().unwrap();

            println!(
                "Bus {:03} Device {:03} ID {:04x}:{:04x}",
                device.bus_number(),
                device.address(),
                device_desc.vendor_id(),
                device_desc.product_id()
            );
        }

        let device_handle: DeviceHandle<rusb::GlobalContext> =
            // Embassy code is: ` embassy_usb::Config::new(0x1209, 0x0001);`
            match rusb::open_device_with_vid_pid(0x1209, 0x0001) {
                None => {
                    println!("STM board not plugged in");
                    return ();
                }
                Some(d) => d,
            };

        println!("STM on {:?}", device_handle);

        // device_handle.read_control(request_type, request, value, index, buf, timeout);
    }
}

#[tokio::main]
async fn main() {
    println!("Hello, world!");
    serial::connect().await;
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
