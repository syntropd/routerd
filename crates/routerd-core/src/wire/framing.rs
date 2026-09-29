//! Syntrop Cluster Mesh Protocol (SCMP) zero-copy binary framing.

use crate::error::{Result, RouterError};
use bytes::{Buf, Bytes, BytesMut};
use rustix::net::sockopt::set_socket_keepalive;
use std::os::fd::AsFd;
use tokio::net::TcpStream;

pub const SCMP_MAGIC: &[u8; 4] = b"SCMP";
pub const SCMP_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 24;
pub const MAX_PAYLOAD_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ScmpMessageType {
    Ping = 1,
    Pong = 2,
    Handshake = 3,
    PrefillChunk = 4,
    ActivationHandoff = 5,
    PrefillAck = 6,
    CircuitHeartbeat = 7,
}

impl ScmpMessageType {
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            1 => Some(Self::Ping),
            2 => Some(Self::Pong),
            3 => Some(Self::Handshake),
            4 => Some(Self::PrefillChunk),
            5 => Some(Self::ActivationHandoff),
            6 => Some(Self::PrefillAck),
            7 => Some(Self::CircuitHeartbeat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScmpHeader {
    pub version: u16,
    pub msg_type: ScmpMessageType,
    pub flags: u32,
    pub payload_len: u32,
    pub seq: u64,
}

impl ScmpHeader {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut buf = [0u8; HEADER_LEN];
        buf[0..4].copy_from_slice(SCMP_MAGIC);
        buf[4..6].copy_from_slice(&self.version.to_be_bytes());
        buf[6..8].copy_from_slice(&(self.msg_type as u16).to_be_bytes());
        buf[8..12].copy_from_slice(&self.flags.to_be_bytes());
        buf[12..16].copy_from_slice(&self.payload_len.to_be_bytes());
        buf[16..24].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    pub fn decode(src: &[u8]) -> Result<Self> {
        if src.len() < HEADER_LEN {
            return Err(RouterError::Config("Truncated SCMP header".into()));
        }
        if &src[0..4] != SCMP_MAGIC {
            return Err(RouterError::Config("Invalid SCMP magic header".into()));
        }
        let version = u16::from_be_bytes([src[4], src[5]]);
        let raw_type = u16::from_be_bytes([src[6], src[7]]);
        let msg_type = ScmpMessageType::from_u16(raw_type)
            .ok_or_else(|| RouterError::Config(format!("Unknown SCMP message type {raw_type}")))?;
        let flags = u32::from_be_bytes([src[8], src[9], src[10], src[11]]);
        let payload_len = u32::from_be_bytes([src[12], src[13], src[14], src[15]]);
        if (payload_len as usize) > MAX_PAYLOAD_LEN {
            return Err(RouterError::Config(format!(
                "SCMP payload length {payload_len} exceeds maximum allowed {MAX_PAYLOAD_LEN}"
            )));
        }
        let seq = u64::from_be_bytes([
            src[16], src[17], src[18], src[19], src[20], src[21], src[22], src[23],
        ]);
        Ok(Self {
            version,
            msg_type,
            flags,
            payload_len,
            seq,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScmpFrame {
    pub header: ScmpHeader,
    pub payload: Bytes,
}

impl ScmpFrame {
    pub fn new(msg_type: ScmpMessageType, seq: u64, payload: Bytes) -> Self {
        Self {
            header: ScmpHeader {
                version: SCMP_VERSION,
                msg_type,
                flags: 0,
                payload_len: payload.len() as u32,
                seq,
            },
            payload,
        }
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(HEADER_LEN + self.payload.len());
        buf.extend_from_slice(&self.header.encode());
        buf.extend_from_slice(&self.payload);
        buf.freeze()
    }

    pub fn decode(src: &mut BytesMut) -> Result<Option<Self>> {
        if src.len() < HEADER_LEN {
            return Ok(None);
        }
        let header = ScmpHeader::decode(&src[..HEADER_LEN])?;
        let total_len = HEADER_LEN + header.payload_len as usize;
        if src.len() < total_len {
            return Ok(None);
        }
        src.advance(HEADER_LEN);
        let payload = src.split_to(header.payload_len as usize).freeze();
        Ok(Some(Self { header, payload }))
    }
}

/// Configure low-latency cluster TCP socket with TCP_NODELAY and SO_KEEPALIVE.
pub fn configure_mesh_tcp(stream: &TcpStream) -> Result<()> {
    stream
        .set_nodelay(true)
        .map_err(|e| RouterError::Config(format!("Failed to set TCP_NODELAY: {e}")))?;
    set_socket_keepalive(stream.as_fd(), true)
        .map_err(|e| RouterError::Config(format!("Failed to set SO_KEEPALIVE: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scmp_frame_encode_decode_roundtrip() {
        let payload = Bytes::from_static(b"test cluster payload data");
        let frame = ScmpFrame::new(ScmpMessageType::PrefillChunk, 42, payload.clone());
        let encoded = frame.encode();

        let mut buf = BytesMut::from(encoded.as_ref());
        let decoded = ScmpFrame::decode(&mut buf).unwrap().expect("frame decoded");
        assert_eq!(decoded.header.version, SCMP_VERSION);
        assert_eq!(decoded.header.msg_type, ScmpMessageType::PrefillChunk);
        assert_eq!(decoded.header.seq, 42);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_scmp_oversized_payload_rejected() {
        let mut raw = [0u8; 24];
        raw[0..4].copy_from_slice(SCMP_MAGIC);
        raw[4..6].copy_from_slice(&1u16.to_be_bytes());
        raw[6..8].copy_from_slice(&(ScmpMessageType::Ping as u16).to_be_bytes());
        // Set payload length to 100 MB (> 64 MB MAX_PAYLOAD_LEN)
        raw[12..16].copy_from_slice(&(100 * 1024 * 1024u32).to_be_bytes());

        let res = ScmpHeader::decode(&raw);
        assert!(res.is_err(), "Payload exceeding 64MB must be rejected");
    }
}

