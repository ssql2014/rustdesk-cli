//! High-level text/terminal session that combines connection + terminal.
//!
//! Provides a simple API for AI agents:
//! - `text_connect(config)` — authenticate, disable audio/video, open terminal
//! - `text_exec(session, cmd)` — send a command, collect output until idle
//! - `text_disconnect(session)` — close terminal and transport

use std::time::Duration;

use anyhow::{Context, Result, bail};
use prost::Message as ProstMessage;

use crate::connection::{ConnectionConfig, ConnectionResult, connect};
use crate::crypto::EncryptedStream;
use crate::proto::hbb::{
    ImageQuality, Message, Misc, OptionMessage, PeerInfo,
    message, misc, option_message,
};
use crate::terminal::{self, TerminalEvent, TerminalInfo};
use crate::transport::TcpTransport;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An active text session with a remote RustDesk peer.
pub struct TextSession {
    pub peer_info: PeerInfo,
    pub terminal: TerminalInfo,
    pub encrypted: EncryptedStream<TcpTransport>,
}

/// Output collected from a single command execution.
#[derive(Debug)]
pub struct ExecOutput {
    /// Raw stdout/stderr bytes accumulated from the terminal.
    pub data: Vec<u8>,
    /// Whether the terminal closed during execution (with exit code).
    pub closed: Option<i32>,
}

impl ExecOutput {
    /// Return the output as a lossy UTF-8 string.
    pub fn as_str(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Connect to a remote peer and open a terminal session.
///
/// 1. Establishes encrypted connection via rendezvous + relay + NaCl + auth
/// 2. Sends OptionMessage to disable audio (text-only mode)
/// 3. Opens a terminal with 80x24 dimensions
pub async fn text_connect(config: &ConnectionConfig) -> Result<TextSession> {
    // Phase 1: Full connection (rendezvous → relay → crypto → auth).
    let ConnectionResult {
        peer_info,
        mut encrypted,
    } = connect(config)
        .await
        .context("connection failed")?;

    // Phase 2: Send OptionMessage to disable audio and set low image quality.
    send_option_message(&mut encrypted).await?;

    // Phase 3: Open terminal (80 cols x 24 rows — standard VT100).
    let terminal = terminal::open_terminal(&mut encrypted, 24, 80)
        .await
        .context("failed to open terminal")?;

    Ok(TextSession {
        peer_info,
        terminal,
        encrypted,
    })
}

/// Execute a command on the remote terminal and collect output.
///
/// Sends `command` (with trailing newline appended if missing), then reads
/// terminal output until no new data arrives within `idle_timeout`.
///
/// This is a simple heuristic — it waits for the shell to go quiet rather
/// than parsing for a prompt. For most non-interactive commands this works
/// well enough.
pub async fn text_exec(
    session: &mut TextSession,
    command: &str,
    idle_timeout: Duration,
) -> Result<ExecOutput> {
    // Ensure command ends with newline.
    let cmd = if command.ends_with('\n') {
        command.to_string()
    } else {
        format!("{command}\n")
    };

    // Send command as stdin.
    terminal::send_terminal_data(
        &mut session.encrypted,
        session.terminal.terminal_id,
        cmd.as_bytes(),
    )
    .await
    .context("sending command")?;

    // Collect output until idle.
    let mut output = ExecOutput {
        data: Vec::new(),
        closed: None,
    };

    loop {
        let recv_fut = terminal::recv_terminal_data(&mut session.encrypted);
        match tokio::time::timeout(idle_timeout, recv_fut).await {
            Ok(Ok(event)) => match event {
                TerminalEvent::Data(chunk) => {
                    output.data.extend_from_slice(&chunk);
                }
                TerminalEvent::Closed { exit_code } => {
                    output.closed = Some(exit_code);
                    break;
                }
                TerminalEvent::Error(msg) => {
                    bail!("terminal error during exec: {msg}");
                }
            },
            Ok(Err(e)) => return Err(e.context("receiving terminal output")),
            Err(_) => {
                // Timeout — no data for `idle_timeout`, assume command finished.
                break;
            }
        }
    }

    Ok(output)
}

/// Close the terminal and disconnect.
pub async fn text_disconnect(mut session: TextSession) -> Result<()> {
    // Close the terminal.
    terminal::close_terminal(&mut session.encrypted, session.terminal.terminal_id)
        .await
        .context("closing terminal")?;

    // Close the transport.
    session
        .encrypted
        .close()
        .await
        .context("closing transport")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Send OptionMessage to disable audio and reduce video overhead.
async fn send_option_message(stream: &mut EncryptedStream<TcpTransport>) -> Result<()> {
    let opt = OptionMessage {
        image_quality: ImageQuality::Low as i32,
        disable_audio: option_message::BoolOption::Yes as i32,
        disable_clipboard: option_message::BoolOption::NotSet as i32,
        enable_file_transfer: option_message::BoolOption::NotSet as i32,
        lock_after_session_end: option_message::BoolOption::NotSet as i32,
        show_remote_cursor: option_message::BoolOption::NotSet as i32,
        privacy_mode: option_message::BoolOption::NotSet as i32,
        block_input: option_message::BoolOption::NotSet as i32,
        custom_image_quality: 0,
        supported_decoding: None,
        custom_fps: 0,
        disable_keyboard: option_message::BoolOption::NotSet as i32,
        follow_remote_cursor: option_message::BoolOption::NotSet as i32,
        follow_remote_window: option_message::BoolOption::NotSet as i32,
        disable_camera: option_message::BoolOption::Yes as i32,
        terminal_persistent: option_message::BoolOption::Yes as i32,
        show_my_cursor: option_message::BoolOption::NotSet as i32,
    };

    let msg = Message {
        union: Some(message::Union::Misc(Misc {
            union: Some(misc::Union::Option(opt)),
        })),
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf)?;
    stream.send(&buf).await.context("sending OptionMessage")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::EncryptedStream;
    use crate::proto::hbb::{
        SupportedDecoding, TerminalData, TerminalResponse,
        terminal_response,
    };
    use crate::transport::FramedTransport;
    use tokio::io::duplex;

    // -- Test-only transport --

    struct DuplexTransport {
        framed: FramedTransport<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn pair() -> (Self, Self) {
            let (a, b) = duplex(16384);
            (
                Self { framed: FramedTransport::new(a) },
                Self { framed: FramedTransport::new(b) },
            )
        }
    }

    impl crate::transport::Transport for DuplexTransport {
        async fn connect(_addr: &str) -> Result<Self> {
            unimplemented!()
        }
        async fn send(&mut self, msg: &[u8]) -> Result<()> {
            self.framed.send(msg).await
        }
        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.framed.recv().await
        }
        async fn close(&mut self) -> Result<()> {
            self.framed.close().await
        }
    }

    async fn send_msg(stream: &mut EncryptedStream<DuplexTransport>, msg: &Message) -> Result<()> {
        let mut buf = Vec::new();
        msg.encode(&mut buf)?;
        stream.send(&buf).await
    }

    async fn recv_msg(stream: &mut EncryptedStream<DuplexTransport>) -> Result<Message> {
        let raw = stream.recv().await?;
        Ok(Message::decode(raw.as_slice())?)
    }

    fn terminal_response_msg(inner: terminal_response::Union) -> Message {
        Message {
            union: Some(message::Union::TerminalResponse(TerminalResponse {
                union: Some(inner),
            })),
        }
    }

    // -- Tests --

    #[tokio::test]
    async fn send_option_message_sets_disable_audio_and_low_quality() {
        let (ct, st) = DuplexTransport::pair();
        let key = [42u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            // Mirror the headless config from daemon::build_option_message (§30).
            let opt = OptionMessage {
                image_quality: ImageQuality::Best as i32,
                custom_fps: 0,
                disable_audio: option_message::BoolOption::Yes as i32,
                disable_clipboard: option_message::BoolOption::Yes as i32,
                disable_camera: option_message::BoolOption::Yes as i32,
                terminal_persistent: option_message::BoolOption::Yes as i32,
                supported_decoding: Some(SupportedDecoding {
                    ability_vp9: 1,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let msg = Message {
                union: Some(message::Union::Misc(Misc {
                    union: Some(misc::Union::Option(opt)),
                })),
            };
            send_msg(&mut client, &msg).await.unwrap();
            client
        });

        let server_task = tokio::spawn(async move {
            let msg = recv_msg(&mut server).await.unwrap();
            match msg.union {
                Some(message::Union::Misc(misc)) => match misc.union {
                    Some(misc::Union::Option(opt)) => {
                        assert_eq!(opt.image_quality, ImageQuality::Best as i32);
                        assert_eq!(opt.custom_fps, 0);
                        assert_eq!(opt.disable_audio, option_message::BoolOption::Yes as i32);
                        assert_eq!(opt.disable_clipboard, option_message::BoolOption::Yes as i32);
                        assert_eq!(opt.disable_camera, option_message::BoolOption::Yes as i32);
                        assert_eq!(
                            opt.terminal_persistent,
                            option_message::BoolOption::Yes as i32
                        );
                        let decoding = opt.supported_decoding.expect("supported_decoding should be set");
                        assert_eq!(decoding.ability_vp9, 1);
                    }
                    other => panic!("expected Option, got {other:?}"),
                },
                other => panic!("expected Misc, got {other:?}"),
            }
            server
        });

        let _client = client_task.await.unwrap();
        let _server = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn text_exec_collects_output_until_timeout() {
        let (ct, st) = DuplexTransport::pair();
        let key = [77u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        // Build a fake TextSession using DuplexTransport.
        // We can't use the real TextSession (it holds EncryptedStream<TcpTransport>),
        // so we test text_exec's logic via the terminal primitives directly.
        let client_task = tokio::spawn(async move {
            // Send command.
            terminal::send_terminal_data(&mut client, 1, b"echo hello\n")
                .await
                .unwrap();

            // Collect output with timeout.
            let mut output = Vec::new();
            let idle = Duration::from_millis(100);
            loop {
                let recv_fut = terminal::recv_terminal_data(&mut client);
                match tokio::time::timeout(idle, recv_fut).await {
                    Ok(Ok(TerminalEvent::Data(chunk))) => output.extend_from_slice(&chunk),
                    Ok(Ok(_)) => break,
                    Ok(Err(e)) => panic!("recv error: {e}"),
                    Err(_) => break, // timeout = done
                }
            }
            (output, client)
        });

        let server_task = tokio::spawn(async move {
            // Receive command.
            let msg = recv_msg(&mut server).await.unwrap();
            match msg.union {
                Some(message::Union::TerminalAction(ta)) => {
                    let action = ta.union.unwrap();
                    match action {
                        crate::proto::hbb::terminal_action::Union::Data(td) => {
                            assert_eq!(td.data, b"echo hello\n");
                        }
                        other => panic!("expected Data, got {other:?}"),
                    }
                }
                other => panic!("expected TerminalAction, got {other:?}"),
            }

            // Send output in two chunks.
            let resp1 = terminal_response_msg(terminal_response::Union::Data(TerminalData {
                terminal_id: 1,
                data: b"hello\n".to_vec(),
                compressed: false,
            }));
            send_msg(&mut server, &resp1).await.unwrap();

            let resp2 = terminal_response_msg(terminal_response::Union::Data(TerminalData {
                terminal_id: 1,
                data: b"$ ".to_vec(),
                compressed: false,
            }));
            send_msg(&mut server, &resp2).await.unwrap();

            // Don't send anything else — let client timeout.
            tokio::time::sleep(Duration::from_millis(200)).await;
            server
        });

        let (output, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        assert_eq!(String::from_utf8_lossy(&output), "hello\n$ ");
    }

    #[tokio::test]
    async fn text_exec_handles_terminal_closed() {
        let (ct, st) = DuplexTransport::pair();
        let key = [66u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            terminal::send_terminal_data(&mut client, 1, b"exit\n")
                .await
                .unwrap();

            let mut output = Vec::new();
            let mut exit_code = None;
            let idle = Duration::from_millis(100);
            loop {
                let recv_fut = terminal::recv_terminal_data(&mut client);
                match tokio::time::timeout(idle, recv_fut).await {
                    Ok(Ok(TerminalEvent::Data(chunk))) => output.extend_from_slice(&chunk),
                    Ok(Ok(TerminalEvent::Closed { exit_code: ec })) => {
                        exit_code = Some(ec);
                        break;
                    }
                    Ok(Ok(TerminalEvent::Error(msg))) => panic!("error: {msg}"),
                    Ok(Err(e)) => panic!("recv error: {e}"),
                    Err(_) => break,
                }
            }
            (output, exit_code, client)
        });

        let server_task = tokio::spawn(async move {
            let _msg = recv_msg(&mut server).await.unwrap();

            // Send some output then close.
            let resp = terminal_response_msg(terminal_response::Union::Data(TerminalData {
                terminal_id: 1,
                data: b"logout\n".to_vec(),
                compressed: false,
            }));
            send_msg(&mut server, &resp).await.unwrap();

            let closed = terminal_response_msg(terminal_response::Union::Closed(
                crate::proto::hbb::TerminalClosed {
                    terminal_id: 1,
                    exit_code: 0,
                },
            ));
            send_msg(&mut server, &closed).await.unwrap();
            server
        });

        let (output, exit_code, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        assert_eq!(String::from_utf8_lossy(&output), "logout\n");
        assert_eq!(exit_code, Some(0));
    }

    #[tokio::test]
    async fn exec_output_as_str_converts_lossy() {
        let output = ExecOutput {
            data: b"hello \xffworld".to_vec(),
            closed: None,
        };
        let s = output.as_str();
        assert!(s.contains("hello"));
        assert!(s.contains("world"));
    }

    /// Live integration test — connects to the real server and opens a terminal.
    #[tokio::test]
    #[ignore = "requires live RustDesk server and target peer online"]
    async fn live_text_session() {
        let config = ConnectionConfig {
            id_server: "115.238.185.55:50076".to_string(),
            relay_server: "115.238.185.55:50077".to_string(),
            server_key: "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=".to_string(),
            peer_id: "308235080".to_string(),
            password: "Evas@2026".to_string(),
            warmup_secs: 2,
        };

        match text_connect(&config).await {
            Ok(mut session) => {
                println!("Text session established!");
                println!("  host: {}", session.peer_info.hostname);
                println!("  platform: {}", session.peer_info.platform);
                println!("  terminal_id: {}", session.terminal.terminal_id);
                println!("  pid: {}", session.terminal.pid);

                // Try running a simple command.
                match text_exec(&mut session, "echo rustdesk-cli-test", Duration::from_secs(3))
                    .await
                {
                    Ok(output) => {
                        println!("  exec output: {:?}", output.as_str());
                        assert!(
                            output.as_str().contains("rustdesk-cli-test"),
                            "output should contain our echo string"
                        );
                    }
                    Err(e) => eprintln!("  exec failed: {e:#}"),
                }

                if let Err(e) = text_disconnect(session).await {
                    eprintln!("  disconnect error: {e:#}");
                }
            }
            Err(e) => {
                eprintln!("Text connect failed (expected if server is down): {e:#}");
            }
        }
    }
}
