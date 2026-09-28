//! Shared telemetry protocol between firmware and ingestor.
//! Uses postcard for serialization + COBS for framing over UART.

#![no_std]
#![cfg_attr(not(test), no_main)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::fmt;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

// Frame counter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct Seq(pub u32);

/// AHRS sensor fusion
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, MaxSize)]
pub struct ImuFusion {
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, MaxSize)]
pub struct TelemetryFrame {
    pub version: u8,
    pub seq: Seq,
    pub uptime_ms: u64,
    pub imu: ImuFusion,
}

impl TelemetryFrame {
    /// Create a new from from (filtered) sensor readings
    pub fn new(seq: u32, uptime_ms: u64, imu: ImuFusion) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            seq: Seq(seq),
            uptime_ms,
            imu,
        }
    }

    /// Encode to COBS + postcard. Return a frame ready for transmission.
    pub fn encode(&self) -> Result<heapless::Vec<u8, 64>, postcard::Error> {
        let mut buf = [0u8; Self::POSTCARD_MAX_SIZE + 4]; // safety margin
        let serialized = postcard::to_slice_cobs(self, &mut buf)?;
        let mut v = heapless::Vec::new();
        v.extend_from_slice(serialized)
            .map_err(|_| postcard::Error::SerializeBufferFull)?;
        Ok(v)
    }

    /// Decode COBS frae back into `TelemetryFrame`.
    ///  **Requires `alloc` feature** (ingestor only)
    #[cfg(feature = "alloc")]
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let decoded = cobs::decode_vec(bytes).map_err(|_| ProtocolError::Cobs)?;
        let frame: TelemetryFrame = postcard::from_bytes(&decoded)?;
        if frame.version != PROTOCOL_VERSION {
            return Err(ProtocolError::VersionMismatch);
        }
        Ok(frame)
    }
}
/// Errors that can occur during (de)serialization.
#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolError {
    Postcard(postcard::Error),
    Cobs,
    VersionMismatch,
}

impl From<postcard::Error> for ProtocolError {
    fn from(e: postcard::Error) -> Self {
        ProtocolError::Postcard(e)
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::Postcard(e) => write!(f, "postcard: {:?}", e),
            ProtocolError::Cobs => f.write_str("COBS decode failed"),
            ProtocolError::VersionMismatch => f.write_str("protocol version mismatch"),
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

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

        let encoded = frame.encode().expect("encode");
        let decoded = TelemetryFrame::decode(&encoded).expect("decode");

        assert_eq!(frame, decoded);
    }
}
