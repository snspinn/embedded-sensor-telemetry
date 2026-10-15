use protocol;

fn main() {
    println!("Hello, world!");
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
