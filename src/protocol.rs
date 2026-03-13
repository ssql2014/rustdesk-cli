//! RustDesk protocol types — placeholder structs matching RustDesk's protobuf definitions.
//! These will be replaced with prost-generated code once we vendor the .proto files.

use serde::{Deserialize, Serialize};

/// Remote peer connection parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub peer_id: String,
    pub password: Option<String>,
    pub server: Option<String>,
}

/// Login request sent to the remote peer after transport is established.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: Vec<u8>,
    pub my_id: String,
    pub my_name: String,
    pub option: LoginOption,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginOption {
    pub video_codec_format: VideoCodecFormat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum VideoCodecFormat {
    VP9,
    VP8,
    AV1,
    H264,
    H265,
}

/// Login response from the remote peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub success: bool,
    pub error: Option<String>,
    pub peer_info: Option<PeerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub username: String,
    pub hostname: String,
    pub displays: Vec<DisplayInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Keyboard event sent to the remote peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key_code: Option<u32>,
    pub characters: Option<String>,
    pub down: bool,
    pub modifiers: KeyModifiers,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Mouse event sent to the remote peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub mask: u32,
    pub is_move: bool,
}

impl MouseEvent {
    pub const BUTTON_LEFT: u32 = 1;
    pub const BUTTON_RIGHT: u32 = 2;
    pub const BUTTON_MIDDLE: u32 = 4;

    pub fn button_mask(button: &str) -> u32 {
        match button {
            "right" => Self::BUTTON_RIGHT,
            "middle" => Self::BUTTON_MIDDLE,
            _ => Self::BUTTON_LEFT,
        }
    }
}

/// A single decoded video frame from the remote display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub codec: VideoCodecFormat,
}

/// Wrapper for all messages sent over the RustDesk connection.
/// Maps to the top-level `Message` in message.proto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProtocolMessage {
    LoginRequest(LoginRequest),
    LoginResponse(LoginResponse),
    KeyEvent(KeyEvent),
    MouseEvent(MouseEvent),
    VideoFrame(VideoFrame),
    Disconnect,
}

impl ProtocolMessage {
    /// Placeholder: serialize to bytes for sending over the wire.
    /// Will be replaced with prost protobuf encoding.
    #[allow(dead_code)]
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Placeholder: deserialize from bytes received over the wire.
    /// Will be replaced with prost protobuf decoding.
    #[allow(dead_code)]
    pub fn decode(data: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(data)?)
    }
}
