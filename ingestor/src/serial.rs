use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use protocol::{ProtocolError, TelemetryFrame};
use tokio::io::AsyncReadExt;
use tokio_serial::{SerialPortBuilderExt, SerialPortType};

// Embassy code is: `embassy_usb::Config::new(0x1209, 0x0001);`
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_PENDING: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub received_at: SystemTime,
    pub frame: TelemetryFrame,
}

#[derive(Default)]
pub struct Deframer {
    pending: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<TelemetryFrame, ProtocolError>> {
        self.pending.extend_from_slice(bytes);

        let mut frames = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == 0) {
            let mut frame: Vec<u8> = self.pending.drain(..=end).collect();
            frames.push(TelemetryFrame::decode(&mut frame));
        }

        if self.pending.len() > MAX_PENDING {
            self.pending.clear();
            frames.push(Err(ProtocolError::Cobs));
        }
        frames
    }
}

pub async fn run(
    port: Option<String>,
    connected: Arc<AtomicBool>,
    mut on_frame: impl FnMut(Received),
) -> ! {
    loop {
        let result = read_port(port.as_deref(), &connected, &mut on_frame).await;
        connected.store(false, Ordering::Relaxed);
        match result {
            Ok(path) => eprintln!("Disconnected from {path}"),
            Err(e) => eprintln!("Serial error: {e}"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn read_port(
    port: Option<&str>,
    connected: &AtomicBool,
    on_frame: &mut impl FnMut(Received),
) -> tokio_serial::Result<String> {
    let path = match port {
        Some(path) => path.to_owned(),
        None => find_port()?,
    };
    let mut stream = tokio_serial::new(&path, 115_200).open_native_async()?;
    connected.store(true, Ordering::Relaxed);
    eprintln!("Connected to {path}");

    let mut deframer = Deframer::default();
    let mut buf = [0u8; 64];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(path);
        }
        let received_at = SystemTime::now();
        for result in deframer.push(&buf[..n]) {
            match result {
                Ok(frame) => on_frame(Received { received_at, frame }),
                Err(e) => eprintln!("Dropping frame: {e}"),
            }
        }
    }
}

fn find_port() -> tokio_serial::Result<String> {
    tokio_serial::available_ports()?
        .into_iter()
        .find(|p| matches!(&p.port_type, SerialPortType::UsbPort(usb) if usb.vid == VID && usb.pid == PID))
        .map(|p| p.port_name)
        .ok_or_else(|| {
            tokio_serial::Error::new(
                tokio_serial::ErrorKind::NoDevice,
                format!("no device {VID:04x}:{PID:04x} found"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::ImuFusion;

    fn frame(seq: u32) -> TelemetryFrame {
        TelemetryFrame::new(
            seq,
            seq as u64 * 100,
            ImuFusion {
                roll: 0.1,
                pitch: 0.01,
                yaw: 0.5,
            },
        )
    }

    #[test]
    fn yields_good_frames_in_order_and_skips_corrupt() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&frame(1).encode().unwrap());
        let corrupt = frame(2).encode().unwrap();
        stream.extend_from_slice(&corrupt[..corrupt.len() / 2]);
        stream.push(0);
        stream.extend_from_slice(&frame(3).encode().unwrap());
        stream.extend_from_slice(&frame(4).encode().unwrap());

        let mut deframer = Deframer::default();
        let results: Vec<_> = stream.chunks(7).flat_map(|c| deframer.push(c)).collect();

        assert_eq!(results.len(), 4);
        assert!(results[1].is_err());
        let good: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
        assert_eq!(good, vec![frame(1), frame(3), frame(4)]);
    }

    #[tokio::test]
    #[ignore = "needs a board running current firmware on USB"]
    async fn board_streams_crc_valid_frames() {
        let path = find_port().expect("board not found");
        let mut stream = tokio_serial::new(&path, 115_200)
            .open_native_async()
            .expect("open port");

        let mut deframer = Deframer::default();
        let mut results = Vec::new();
        let mut buf = [0u8; 64];
        while results.len() < 21 {
            let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
                .await
                .expect("no data from board")
                .expect("read");
            assert_ne!(n, 0, "board disconnected");
            results.extend(deframer.push(&buf[..n]));
        }

        // The first frame may be cut short by connecting mid-stream.
        let frames: Vec<TelemetryFrame> = results[1..]
            .iter()
            .cloned()
            .collect::<Result<_, _>>()
            .expect("frame failed decode or CRC check");
        assert!(frames.windows(2).all(|w| w[1].seq.0 > w[0].seq.0));
    }
}
