//! Shared telemetry protocol between firmware and ingestor.
//! Uses postcard for serialization, a CRC-32 for integrity, and COBS for framing over UART.

#![no_std]

use core::fmt;
use crc::{Crc, CRC_32_ISCSI};
use postcard::experimental::max_size::MaxSize;
use postcard::ser_flavors::{crc::CrcModifier, Cobs, Slice};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 2;

static CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
const CRC_LEN: usize = 4;

// Frame counter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Seq(pub u32);

/// AHRS sensor fusion
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ImuFusion {
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

    /// COBS-encoded postcard payload plus CRC, including the `0x00` delimiter.
    pub const ENCODED_MAX_SIZE: usize =
        cobs::max_encoding_length(Self::POSTCARD_MAX_SIZE + CRC_LEN) + 1;

    /// Encode to postcard + CRC-32 + COBS. Return a frame ready for transmission.
    pub fn encode(&self) -> Result<heapless::Vec<u8, 64>, postcard::Error> {
        let mut buf = [0u8; Self::ENCODED_MAX_SIZE];
        // CRC must wrap COBS so the checksum is appended before framing, never after the delimiter.
        let flavor = CrcModifier::new(Cobs::try_new(Slice::new(&mut buf))?, CRC.digest());
        let serialized = postcard::serialize_with_flavor(self, flavor)?;
        let mut v = heapless::Vec::new();
        v.extend_from_slice(serialized)
            .map_err(|_| postcard::Error::SerializeBufferFull)?;
        Ok(v)
    }

    /// Decode a COBS frame and verify its CRC back into `TelemetryFrame`.
    pub fn decode(bytes: &mut [u8]) -> Result<Self, ProtocolError> {
        let len = cobs::decode_in_place(bytes).map_err(|_| ProtocolError::Cobs)?;
        let frame: TelemetryFrame = postcard::from_bytes_crc32(&bytes[..len], CRC.digest())?;
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

// Human-readable Display for the host (std) side only
#[cfg(feature = "std")]
impl fmt::Display for TelemetryFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TelemetryFrame(v={}, seq={}, t={}ms, imu=[{:?}])",
            self.version, self.seq.0, self.uptime_ms, self.imu
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> TelemetryFrame {
        TelemetryFrame::new(
            42,
            12345678,
            ImuFusion {
                roll: 0.1,
                pitch: 0.01,
                yaw: 0.5,
            },
        )
    }

    #[test]
    fn roundtrip() {
        let frame = frame();
        let encoded = &mut frame.encode().expect("encode");
        let bytes: &mut [u8] = &mut encoded[..];
        let decoded = TelemetryFrame::decode(bytes).expect("decode");

        assert_eq!(frame, decoded);
    }

    #[test]
    fn rejects_corrupted_payload() {
        let mut encoded = frame().encode().expect("encode");
        let len = cobs::decode_in_place(&mut encoded).expect("cobs decode");
        encoded[len - CRC_LEN - 1] ^= 0x01;

        let mut reframed = [0u8; 64];
        let n = cobs::encode(&encoded[..len], &mut reframed);

        assert_eq!(
            TelemetryFrame::decode(&mut reframed[..=n]),
            Err(ProtocolError::Postcard(postcard::Error::DeserializeBadCrc))
        );
    }
}
