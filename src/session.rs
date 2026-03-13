//! Session state management.
//! Tracks connection state and translates CLI commands into protocol messages.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::protocol::{
    ConnectionConfig, DisplayInfo, KeyEvent, KeyModifiers, MouseEvent, ProtocolMessage,
};

/// Connection state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

/// Session holds all state for an active remote connection.
#[derive(Debug)]
pub struct Session {
    pub state: ConnectionState,
    pub config: Option<ConnectionConfig>,
    pub peer_info: Option<PeerInfoState>,
}

/// Resolved peer info after successful login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfoState {
    pub peer_id: String,
    pub username: String,
    pub hostname: String,
    pub displays: Vec<DisplayInfo>,
}

/// Commands that the CLI sends to the daemon over UDS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionCommand {
    Connect {
        peer_id: String,
        password: Option<String>,
        server: Option<String>,
    },
    Disconnect,
    Capture {
        output: String,
    },
    Type {
        text: String,
    },
    Key {
        key: String,
    },
    Click {
        x: i32,
        y: i32,
        button: String,
    },
    Move {
        x: i32,
        y: i32,
    },
    Status,
}

/// Response from the daemon back to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub success: bool,
    pub message: Option<String>,
    pub data: Option<serde_json::Value>,
}

impl SessionResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(message.into()),
            data: None,
        }
    }

    pub fn ok_with_data(message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: true,
            message: Some(message.into()),
            data: Some(data),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: Some(message.into()),
            data: None,
        }
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            config: None,
            peer_info: None,
        }
    }

    /// Dispatch a command and return the protocol message(s) to send,
    /// plus the response to return to the CLI.
    pub fn dispatch(&mut self, cmd: SessionCommand) -> Result<(SessionResponse, Vec<ProtocolMessage>)> {
        match cmd {
            SessionCommand::Connect { peer_id, password, server } => {
                if self.state == ConnectionState::Connected {
                    return Ok((
                        SessionResponse::error("Already connected"),
                        vec![],
                    ));
                }
                self.state = ConnectionState::Connecting;
                self.config = Some(ConnectionConfig {
                    peer_id: peer_id.clone(),
                    password,
                    server,
                });
                // TODO: Initiate actual RustDesk connection here.
                // For now, stub as "connected".
                self.state = ConnectionState::Connected;
                Ok((
                    SessionResponse::ok(format!("Connected to {peer_id}")),
                    vec![],
                ))
            }

            SessionCommand::Disconnect => {
                self.require_connected()?;
                let peer_id = self.config.as_ref().map(|c| c.peer_id.clone()).unwrap_or_default();
                self.state = ConnectionState::Disconnected;
                self.config = None;
                self.peer_info = None;
                Ok((
                    SessionResponse::ok(format!("Disconnected from {peer_id}")),
                    vec![ProtocolMessage::Disconnect],
                ))
            }

            SessionCommand::Capture { output } => {
                self.require_connected()?;
                // TODO: Request a keyframe from the video stream, decode, encode as PNG.
                Ok((
                    SessionResponse::ok(format!("Screenshot saved to {output}")),
                    vec![],
                ))
            }

            SessionCommand::Type { text } => {
                self.require_connected()?;
                let mut messages = Vec::new();
                for ch in text.chars() {
                    messages.push(ProtocolMessage::KeyEvent(KeyEvent {
                        key_code: None,
                        characters: Some(ch.to_string()),
                        down: true,
                        modifiers: KeyModifiers::default(),
                    }));
                    messages.push(ProtocolMessage::KeyEvent(KeyEvent {
                        key_code: None,
                        characters: Some(ch.to_string()),
                        down: false,
                        modifiers: KeyModifiers::default(),
                    }));
                }
                Ok((
                    SessionResponse::ok(format!("Typed {} characters", text.len())),
                    messages,
                ))
            }

            SessionCommand::Key { key } => {
                self.require_connected()?;
                let messages = vec![
                    ProtocolMessage::KeyEvent(KeyEvent {
                        key_code: None,
                        characters: Some(key.clone()),
                        down: true,
                        modifiers: KeyModifiers::default(),
                    }),
                    ProtocolMessage::KeyEvent(KeyEvent {
                        key_code: None,
                        characters: Some(key),
                        down: false,
                        modifiers: KeyModifiers::default(),
                    }),
                ];
                Ok((SessionResponse::ok("Key sent"), messages))
            }

            SessionCommand::Click { x, y, button } => {
                self.require_connected()?;
                let mask = MouseEvent::button_mask(&button);
                let messages = vec![
                    ProtocolMessage::MouseEvent(MouseEvent {
                        x,
                        y,
                        mask,
                        is_move: false,
                    }),
                    ProtocolMessage::MouseEvent(MouseEvent {
                        x,
                        y,
                        mask: 0,
                        is_move: false,
                    }),
                ];
                Ok((
                    SessionResponse::ok(format!("{button} click at ({x}, {y})")),
                    messages,
                ))
            }

            SessionCommand::Move { x, y } => {
                self.require_connected()?;
                let messages = vec![ProtocolMessage::MouseEvent(MouseEvent {
                    x,
                    y,
                    mask: 0,
                    is_move: true,
                })];
                Ok((
                    SessionResponse::ok(format!("Moved to ({x}, {y})")),
                    messages,
                ))
            }

            SessionCommand::Status => {
                let data = serde_json::json!({
                    "state": self.state,
                    "peer_id": self.config.as_ref().map(|c| &c.peer_id),
                    "peer_info": self.peer_info,
                });
                Ok((SessionResponse::ok_with_data("Status", data), vec![]))
            }
        }
    }

    fn require_connected(&self) -> Result<()> {
        if self.state != ConnectionState::Connected {
            bail!("No active session. Run `rustdesk-cli connect` first.");
        }
        Ok(())
    }
}
