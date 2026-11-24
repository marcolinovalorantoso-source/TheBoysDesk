//! Binary wire protocol and framing for P2P remote desktop sessions.
//!
//! All control, video, and input messages are serialized with `postcard` and
//! framed over QUIC streams with a 4-byte big-endian length prefix.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Current wire protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Maximum allowed frame payload size (16 MiB) for video frames.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Maximum allowed control frame size (64 KiB) to prevent pre-auth OOM DoS.
pub const MAX_CONTROL_FRAME_SIZE: usize = 64 * 1024;

/// Maximum allowed input frame size (4 KiB).
pub const MAX_INPUT_FRAME_SIZE: usize = 4 * 1024;

/// Default UDP port used for QUIC P2P connections.
pub const DEFAULT_QUIC_PORT: u16 = 21116;

/// Default UDP port used for LAN discovery broadcasts.
pub const DEFAULT_DISCOVERY_PORT: u16 = 21117;

/// Magic header for LAN discovery packets.
pub const DISCOVERY_MAGIC: [u8; 4] = *b"ADPR";

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("serialization failed: {0}")]
    Serialize(#[from] postcard::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame too large: {0} bytes (max {MAX_FRAME_SIZE})")]
    FrameTooLarge(usize),
    #[error("unknown stream kind: 0x{0:02x}")]
    UnknownStreamKind(u8),
}

/// Identifies the purpose of an opened QUIC stream.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Control = 0x01,
    Video = 0x02,
    Input = 0x03,
}

impl TryFrom<u8> for StreamKind {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Control),
            0x02 => Ok(Self::Video),
            0x03 => Ok(Self::Input),
            other => Err(ProtocolError::UnknownStreamKind(other)),
        }
    }
}

/// Metadata describing an available monitor on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

/// Control-channel messages exchanged on the bidirectional `Control` stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlMessage {
    Hello {
        protocol_version: u16,
        client_name: String,
        device_id: String,
    },
    AuthChallenge {
        nonce: [u8; 32],
        host_name: String,
        device_id: String,
        requires_pin: bool,
        requires_manual_accept: bool,
    },
    AuthResponse {
        /// SHA-256(nonce || pin_bytes) so the PIN is never sent in plaintext.
        pin_proof: [u8; 32],
    },
    /// Sent by the host once the PIN is verified and the consent popup is shown.
    AuthPendingConsent {
        host_name: String,
    },
    AuthResult {
        accepted: bool,
        reason: Option<String>,
        monitors: Vec<MonitorInfo>,
    },
    SelectMonitor {
        monitor_id: u32,
    },
    RequestKeyframe,
    QualityHint {
        max_fps: u8,
        target_bitrate_kbps: u32,
    },
    Ping {
        seq: u64,
        sent_us: u64,
    },
    Pong {
        seq: u64,
        sent_us: u64,
    },
    Disconnect {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
}

/// Encoded video frame sent on the unidirectional `Video` stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFrame {
    pub frame_id: u64,
    pub timestamp_us: u64,
    pub monitor_id: u32,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub codec: VideoCodec,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// Remote input event sent on the prioritized `Input` stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    /// Normalized coordinates in [0.0, 1.0] relative to the active monitor.
    MouseMove {
        x_norm: f32,
        y_norm: f32,
        monitor_id: u32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    MouseWheel {
        delta_x: f32,
        delta_y: f32,
    },
    Key {
        scan_code: u16,
        extended: bool,
        pressed: bool,
    },
    ReleaseAllKeys,
}

/// UDP broadcast/multicast packet for automatic LAN peer discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryBeacon {
    pub magic: [u8; 4],
    pub device_id: String,
    pub device_name: String,
    pub quic_port: u16,
}

/// Serializes any `Serialize` message into a length-prefixed buffer.
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = postcard::to_allocvec(msg)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge(payload.len()));
    }
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Deserializes a payload slice (without the 4-byte length prefix).
pub fn decode_payload<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, ProtocolError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// Writes a length-prefixed frame to an async writer.
pub async fn write_frame<W, T>(writer: &mut W, msg: &T) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let buf = encode_frame(msg)?;
    writer.write_all(&buf).await?;
    Ok(())
}

/// Reads a length-prefixed frame from an async reader, enforcing `MAX_FRAME_SIZE`.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<T, ProtocolError>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    read_frame_limited(reader, MAX_FRAME_SIZE).await
}

/// Reads a length-prefixed frame from an async reader with a custom size cap.
pub async fn read_frame_limited<R, T>(reader: &mut R, max_size: usize) -> Result<T, ProtocolError>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > max_size {
        return Err(ProtocolError::FrameTooLarge(len));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(postcard::from_bytes(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_control_message() {
        let msg = ControlMessage::AuthResult {
            accepted: true,
            reason: None,
            monitors: vec![MonitorInfo {
                id: 0,
                name: "DISPLAY1".into(),
                width: 1920,
                height: 1080,
                is_primary: true,
            }],
        };
        let encoded = encode_frame(&msg).unwrap();
        let len = u32::from_be_bytes(encoded[..4].try_into().unwrap()) as usize;
        assert_eq!(len, encoded.len() - 4);
        let decoded: ControlMessage = decode_payload(&encoded[4..]).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn roundtrip_input_event() {
        let ev = InputEvent::MouseMove {
            x_norm: 0.5,
            y_norm: 0.25,
            monitor_id: 1,
        };
        let encoded = encode_frame(&ev).unwrap();
        let decoded: InputEvent = decode_payload(&encoded[4..]).unwrap();
        assert_eq!(ev, decoded);
    }

    #[tokio::test]
    async fn async_read_write_frame() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let frame = VideoFrame {
            frame_id: 42,
            timestamp_us: 123_456,
            monitor_id: 0,
            width: 1280,
            height: 720,
            is_keyframe: true,
            codec: VideoCodec::H264,
            data: vec![0, 0, 0, 1, 0x67],
        };
        write_frame(&mut client, &frame).await.unwrap();
        let read_back: VideoFrame = read_frame(&mut server).await.unwrap();
        assert_eq!(frame, read_back);
    }

    #[tokio::test]
    async fn rejects_oversized_control_frame() {
        use tokio::io::AsyncWriteExt;
        let (mut client, mut server) = tokio::io::duplex(64);
        let fake_len = ((MAX_CONTROL_FRAME_SIZE + 1) as u32).to_be_bytes();
        client.write_all(&fake_len).await.unwrap();
        let err = read_frame_limited::<_, ControlMessage>(&mut server, MAX_CONTROL_FRAME_SIZE)
            .await
            .unwrap_err();
        assert!(matches!(err, ProtocolError::FrameTooLarge(_)));
    }
}
