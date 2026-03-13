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
    Shell,
    Exec {
        command: String,
    },
    ClipboardGet,
    ClipboardSet {
        text: String,
    },
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
        double: bool,
    },
    Drag {
        x: i32,
        y: i32,
        x2: i32,
        y2: i32,
        button: String,
    },
    Scroll {
        x: i32,
        y: i32,
        delta: i32,
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

            SessionCommand::Shell => {
                self.require_connected()?;
                Ok((
                    SessionResponse::ok_with_data(
                        "Opened remote shell",
                        serde_json::json!({
                            "mode": "interactive",
                        }),
                    ),
                    vec![],
                ))
            }

            SessionCommand::Exec { command } => {
                self.require_connected()?;
                Ok((
                    SessionResponse::ok_with_data(
                        format!("Executed `{command}`"),
                        serde_json::json!({
                            "command": command,
                            "stdout": "stub exec output",
                            "stderr": "",
                            "exit_code": 0,
                        }),
                    ),
                    vec![],
                ))
            }

            SessionCommand::ClipboardGet => {
                self.require_connected()?;
                Ok((
                    SessionResponse::ok_with_data(
                        "Clipboard text retrieved",
                        serde_json::json!({
                            "text": "stub clipboard text",
                        }),
                    ),
                    vec![],
                ))
            }

            SessionCommand::ClipboardSet { text } => {
                self.require_connected()?;
                Ok((
                    SessionResponse::ok_with_data(
                        "Clipboard text updated",
                        serde_json::json!({
                            "chars": text.chars().count(),
                            "redacted": true,
                        }),
                    ),
                    vec![],
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

            SessionCommand::Click { x, y, button, double } => {
                self.require_connected()?;
                let mask = MouseEvent::button_mask(&button);
                let click_pair = vec![
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
                let messages = if double {
                    let mut msgs = click_pair.clone();
                    msgs.extend(click_pair);
                    msgs
                } else {
                    click_pair
                };
                Ok((
                    SessionResponse::ok(format!("{button} click at ({x}, {y})")),
                    messages,
                ))
            }

            SessionCommand::Drag { x, y, x2, y2, button } => {
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
                        x: x2,
                        y: y2,
                        mask,
                        is_move: true,
                    }),
                    ProtocolMessage::MouseEvent(MouseEvent {
                        x: x2,
                        y: y2,
                        mask: 0,
                        is_move: false,
                    }),
                ];
                Ok((
                    SessionResponse::ok(format!("{button} drag from ({x}, {y}) to ({x2}, {y2})")),
                    messages,
                ))
            }

            SessionCommand::Scroll { x, y, delta } => {
                self.require_connected()?;
                let mask = if delta >= 0 {
                    MouseEvent::SCROLL_UP
                } else {
                    MouseEvent::SCROLL_DOWN
                };
                let mut messages = Vec::new();
                for _ in 0..delta.abs() {
                    messages.push(ProtocolMessage::MouseEvent(MouseEvent {
                        x,
                        y,
                        mask,
                        is_move: false,
                    }));
                    messages.push(ProtocolMessage::MouseEvent(MouseEvent {
                        x,
                        y,
                        mask: 0,
                        is_move: false,
                    }));
                }
                Ok((
                    SessionResponse::ok(format!("Scrolled {delta} at ({x}, {y})")),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn connected_session() -> Session {
        let mut session = Session::new();
        let _ = session.dispatch(SessionCommand::Connect {
            peer_id: "test".to_string(),
            password: None,
            server: None,
        }).expect("connect should succeed");
        session
    }

    #[test]
    fn new_session_starts_disconnected() {
        let session = Session::new();

        assert_eq!(session.state, ConnectionState::Disconnected);
        assert!(session.config.is_none());
        assert!(session.peer_info.is_none());
    }

    #[test]
    fn connect_transitions_to_connected_and_returns_success() {
        let mut session = Session::new();

        let (response, messages) = session.dispatch(SessionCommand::Connect {
            peer_id: "test".to_string(),
            password: None,
            server: None,
        }).expect("connect should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Connected to test"));
        assert!(messages.is_empty());
        assert_eq!(session.state, ConnectionState::Connected);
        assert_eq!(
            session.config.as_ref().map(|config| config.peer_id.as_str()),
            Some("test")
        );
    }

    #[test]
    fn disconnect_from_connected_returns_disconnect_message() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Disconnect)
            .expect("disconnect should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Disconnected from test"));
        assert_eq!(messages.len(), 1);
        assert!(matches!(messages[0], ProtocolMessage::Disconnect));
        assert_eq!(session.state, ConnectionState::Disconnected);
        assert!(session.config.is_none());
        assert!(session.peer_info.is_none());
    }

    #[test]
    fn shell_returns_interactive_mode_payload() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Shell)
            .expect("shell should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Opened remote shell"));
        assert!(messages.is_empty());

        let data = response.data.expect("shell should include data");
        assert_eq!(data["mode"], serde_json::json!("interactive"));
    }

    #[test]
    fn exec_returns_stub_output_payload() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Exec {
                command: "whoami".to_string(),
            })
            .expect("exec should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Executed `whoami`"));
        assert!(messages.is_empty());

        let data = response.data.expect("exec should include data");
        assert_eq!(data["command"], serde_json::json!("whoami"));
        assert_eq!(data["stdout"], serde_json::json!("stub exec output"));
        assert_eq!(data["stderr"], serde_json::json!(""));
        assert_eq!(data["exit_code"], serde_json::json!(0));
    }

    #[test]
    fn clipboard_get_returns_stub_text() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::ClipboardGet)
            .expect("clipboard get should succeed");

        assert!(response.success);
        assert_eq!(
            response.message.as_deref(),
            Some("Clipboard text retrieved")
        );
        assert!(messages.is_empty());

        let data = response.data.expect("clipboard get should include data");
        assert_eq!(data["text"], serde_json::json!("stub clipboard text"));
    }

    #[test]
    fn clipboard_set_reports_redacted_character_count() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::ClipboardSet {
                text: "copied text".to_string(),
            })
            .expect("clipboard set should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Clipboard text updated"));
        assert!(messages.is_empty());

        let data = response.data.expect("clipboard set should include data");
        assert_eq!(data["chars"], serde_json::json!(11));
        assert_eq!(data["redacted"], serde_json::json!(true));
    }

    #[test]
    fn type_generates_key_event_sequence_for_each_character() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Type {
                text: "hello".to_string(),
            })
            .expect("type should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Typed 5 characters"));
        assert_eq!(messages.len(), 10);

        for (index, ch) in "hello".chars().enumerate() {
            let expected = ch.to_string();
            let down = &messages[index * 2];
            let up = &messages[index * 2 + 1];

            match down {
                ProtocolMessage::KeyEvent(event) => {
                    assert_eq!(event.characters.as_deref(), Some(expected.as_str()));
                    assert!(event.down);
                    assert!(event.key_code.is_none());
                    assert!(!event.modifiers.shift);
                    assert!(!event.modifiers.ctrl);
                    assert!(!event.modifiers.alt);
                    assert!(!event.modifiers.meta);
                }
                other => panic!("expected key down event, got {other:?}"),
            }

            match up {
                ProtocolMessage::KeyEvent(event) => {
                    assert_eq!(event.characters.as_deref(), Some(expected.as_str()));
                    assert!(!event.down);
                    assert!(event.key_code.is_none());
                }
                other => panic!("expected key up event, got {other:?}"),
            }
        }
    }

    #[test]
    fn click_generates_left_button_press_and_release() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Click {
                x: 100,
                y: 200,
                button: "left".to_string(),
                double: false,
            })
            .expect("click should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("left click at (100, 200)"));
        assert_eq!(messages.len(), 2);

        match &messages[0] {
            ProtocolMessage::MouseEvent(event) => {
                assert_eq!(event.x, 100);
                assert_eq!(event.y, 200);
                assert_eq!(event.mask, MouseEvent::BUTTON_LEFT);
                assert!(!event.is_move);
            }
            other => panic!("expected mouse down event, got {other:?}"),
        }

        match &messages[1] {
            ProtocolMessage::MouseEvent(event) => {
                assert_eq!(event.x, 100);
                assert_eq!(event.y, 200);
                assert_eq!(event.mask, 0);
                assert!(!event.is_move);
            }
            other => panic!("expected mouse up event, got {other:?}"),
        }
    }

    #[test]
    fn drag_generates_press_move_release_sequence() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Drag {
                x: 100,
                y: 200,
                x2: 300,
                y2: 400,
                button: "left".to_string(),
            })
            .expect("drag should succeed");

        assert!(response.success);
        assert_eq!(
            response.message.as_deref(),
            Some("left drag from (100, 200) to (300, 400)")
        );
        assert_eq!(messages.len(), 3);

        match &messages[0] {
            ProtocolMessage::MouseEvent(event) => {
                assert_eq!(event.x, 100);
                assert_eq!(event.y, 200);
                assert_eq!(event.mask, MouseEvent::BUTTON_LEFT);
                assert!(!event.is_move);
            }
            other => panic!("expected drag press event, got {other:?}"),
        }

        match &messages[1] {
            ProtocolMessage::MouseEvent(event) => {
                assert_eq!(event.x, 300);
                assert_eq!(event.y, 400);
                assert_eq!(event.mask, MouseEvent::BUTTON_LEFT);
                assert!(event.is_move);
            }
            other => panic!("expected drag move event, got {other:?}"),
        }

        match &messages[2] {
            ProtocolMessage::MouseEvent(event) => {
                assert_eq!(event.x, 300);
                assert_eq!(event.y, 400);
                assert_eq!(event.mask, 0);
                assert!(!event.is_move);
            }
            other => panic!("expected drag release event, got {other:?}"),
        }
    }

    #[test]
    fn scroll_generates_scroll_up_events_for_positive_delta() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Scroll {
                x: 50,
                y: 75,
                delta: 2,
            })
            .expect("scroll should succeed");

        assert!(response.success);
        assert_eq!(response.message.as_deref(), Some("Scrolled 2 at (50, 75)"));
        assert_eq!(messages.len(), 4);

        for (index, message) in messages.iter().enumerate() {
            match message {
                ProtocolMessage::MouseEvent(event) => {
                    assert_eq!(event.x, 50);
                    assert_eq!(event.y, 75);
                    assert!(!event.is_move);
                    if index % 2 == 0 {
                        assert_eq!(event.mask, MouseEvent::SCROLL_UP);
                    } else {
                        assert_eq!(event.mask, 0);
                    }
                }
                other => panic!("expected scroll mouse event, got {other:?}"),
            }
        }
    }

    #[test]
    fn status_returns_data_with_state_and_peer_id() {
        let mut session = connected_session();

        let (response, messages) = session
            .dispatch(SessionCommand::Status)
            .expect("status should succeed");

        assert!(response.success);
        assert!(messages.is_empty());

        let data = response.data.expect("status should include data");
        assert_eq!(data["state"], serde_json::json!("Connected"));
        assert_eq!(data["peer_id"], serde_json::json!("test"));
        assert!(data.get("peer_info").is_some());
    }

    #[test]
    fn disconnected_session_rejects_commands_that_require_connection() {
        let commands = [
            SessionCommand::Disconnect,
            SessionCommand::Shell,
            SessionCommand::Exec {
                command: "pwd".to_string(),
            },
            SessionCommand::ClipboardGet,
            SessionCommand::ClipboardSet {
                text: "clipboard".to_string(),
            },
            SessionCommand::Capture {
                output: "shot.png".to_string(),
            },
            SessionCommand::Type {
                text: "hello".to_string(),
            },
            SessionCommand::Key {
                key: "enter".to_string(),
            },
            SessionCommand::Click {
                x: 100,
                y: 200,
                button: "left".to_string(),
                double: false,
            },
            SessionCommand::Drag {
                x: 100,
                y: 200,
                x2: 300,
                y2: 400,
                button: "left".to_string(),
            },
            SessionCommand::Scroll {
                x: 100,
                y: 200,
                delta: -1,
            },
            SessionCommand::Move { x: 100, y: 200 },
        ];

        for command in commands {
            let mut session = Session::new();
            let error = session.dispatch(command).expect_err("command should fail");
            assert_eq!(
                error.to_string(),
                "No active session. Run `rustdesk-cli connect` first."
            );
            assert_eq!(session.state, ConnectionState::Disconnected);
        }
    }
}
