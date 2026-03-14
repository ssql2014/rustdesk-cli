//! Remote terminal (PTY) session over an encrypted RustDesk connection.
//!
//! Wraps `TerminalAction` / `TerminalResponse` protobuf messages to provide
//! a simple async API for opening a remote shell, sending stdin, receiving
//! stdout, resizing the PTY, and closing the session.

use anyhow::{Context, Result, bail};
use prost::Message as ProstMessage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, timeout};

use crate::connection::{self, ConnectionConfig, ConnectionResult};
use crate::crypto::EncryptedStream;
use crate::proto::hbb::{
    CloseTerminal, ConnType, Message, OpenTerminal, PeerInfo, ResizeTerminal,
    Terminal as LoginTerminal, TerminalAction, TerminalData, login_request, message,
    terminal_action, terminal_response,
};
use crate::transport::{TcpTransport, Transport};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Information returned when a terminal is successfully opened.
#[derive(Debug)]
pub struct TerminalInfo {
    pub terminal_id: i32,
    pub pid: u32,
    pub service_id: String,
}

/// A live terminal-mode connection after login and `OpenTerminal`.
pub struct TerminalConnection {
    pub peer_info: PeerInfo,
    pub terminal_info: TerminalInfo,
    pub encrypted: EncryptedStream<TcpTransport>,
}

/// An event received from the remote terminal.
#[derive(Debug)]
pub enum TerminalEvent {
    /// Stdout/stderr data from the remote shell.
    Data(Vec<u8>),
    /// The terminal process exited.
    Closed { exit_code: i32 },
    /// An error occurred on the remote side.
    Error(String),
}

fn default_terminal_size() -> (u32, u32) {
    let rows = std::env::var("LINES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(24);
    let cols = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(80);
    (rows, cols)
}

const TERMINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const OPEN_TERMINAL_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_TERMINAL_OPENED_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

fn terminal_info_from_opened(opened: crate::proto::hbb::TerminalOpened) -> Result<TerminalInfo> {
    if !opened.success {
        bail!("remote refused to open terminal: {}", opened.message);
    }

    Ok(TerminalInfo {
        terminal_id: opened.terminal_id,
        pid: opened.pid,
        service_id: opened.service_id,
    })
}

// ---------------------------------------------------------------------------
// Send helpers
// ---------------------------------------------------------------------------

/// Encode a `TerminalAction` inside a `Message` and send it encrypted.
async fn send_action<T: Transport>(
    stream: &mut EncryptedStream<T>,
    action: terminal_action::Union,
) -> Result<()> {
    let msg = Message {
        union: Some(message::Union::TerminalAction(TerminalAction {
            union: Some(action),
        })),
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf)?;
    stream.send(&buf).await
}

// ---------------------------------------------------------------------------
// Receive helper
// ---------------------------------------------------------------------------

/// Receive the next `TerminalResponse` from the stream.
///
/// Skips non-terminal messages (e.g. video frames the server may still send).
/// Returns `None` if the stream is closed before a terminal message arrives.
async fn recv_terminal_response<T: Transport>(
    stream: &mut EncryptedStream<T>,
) -> Result<terminal_response::Union> {
    timeout(TERMINAL_RESPONSE_TIMEOUT, async {
        loop {
            let raw = stream.recv().await.context("reading terminal response")?;
            let msg = Message::decode(raw.as_slice()).context("decoding Message")?;

            match msg.union {
                Some(message::Union::TerminalResponse(tr)) => match tr.union {
                    Some(inner) => return Ok(inner),
                    None => bail!("TerminalResponse with empty union"),
                },
                // Ignore non-terminal messages (video frames, test delays, etc.)
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!(
        "timed out waiting for TerminalResponse after {}s",
        TERMINAL_RESPONSE_TIMEOUT.as_secs()
    ))?
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Establish a terminal-mode RustDesk connection and open the remote PTY.
pub async fn connect_terminal(config: &ConnectionConfig) -> Result<TerminalConnection> {
    let ConnectionResult {
        peer_info,
        mut encrypted,
    } = connection::connect_with_mode(
        config,
        ConnType::Terminal,
        Some(login_request::Union::Terminal(LoginTerminal {
            service_id: String::new(),
        })),
    )
    .await?;

    let terminal_info = match timeout(
        INITIAL_TERMINAL_OPENED_PROBE_TIMEOUT,
        recv_terminal_response(&mut encrypted),
    )
    .await
    {
        Ok(Ok(terminal_response::Union::Opened(opened))) => {
            terminal_info_from_opened(opened)
                .context("processing initial TerminalOpened after login")?
        }
        Ok(Ok(terminal_response::Union::Error(e))) => {
            bail!("terminal error after login: {}", e.message);
        }
        Ok(Ok(other)) => {
            bail!("expected TerminalOpened after terminal login, got {other:?}");
        }
        Ok(Err(e)) => return Err(e).context("waiting for initial TerminalOpened"),
        Err(_) => {
            let (rows, cols) = default_terminal_size();
            open_terminal(&mut encrypted, rows, cols)
                .await
                .context("opening remote terminal")?
        }
    };

    Ok(TerminalConnection {
        peer_info,
        terminal_info,
        encrypted,
    })
}

/// Run an interactive terminal session, wiring local stdin/stdout to the peer.
pub async fn run_terminal_session(config: &ConnectionConfig) -> Result<()> {
    let mut connection = connect_terminal(config).await?;
    let terminal_id = connection.terminal_info.terminal_id;

    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = [0_u8; 4096];

        loop {
            match stdin.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if stdin_tx.send(buf[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut stdout = tokio::io::stdout();

    loop {
        enum Event {
            Stdin(Option<Vec<u8>>),
            Terminal(Result<TerminalEvent>),
        }

        let event = tokio::select! {
            chunk = stdin_rx.recv() => Event::Stdin(chunk),
            result = recv_terminal_data(&mut connection.encrypted) => Event::Terminal(result),
        };

        match event {
            Event::Stdin(Some(data)) => {
                send_terminal_data(&mut connection.encrypted, terminal_id, &data).await?;
            }
            Event::Stdin(None) => {
                let _ = close_terminal(&mut connection.encrypted, terminal_id).await;
                break;
            }
            Event::Terminal(Ok(TerminalEvent::Data(data))) => {
                stdout
                    .write_all(&data)
                    .await
                    .context("writing terminal stdout")?;
                stdout
                    .flush()
                    .await
                    .context("flushing terminal stdout")?;
            }
            Event::Terminal(Ok(TerminalEvent::Closed { .. })) => break,
            Event::Terminal(Ok(TerminalEvent::Error(msg))) => {
                bail!("terminal error: {msg}");
            }
            Event::Terminal(Err(e)) => {
                return Err(e).context("receiving terminal output");
            }
        }
    }

    Ok(())
}

/// Open a remote terminal with the given dimensions.
///
/// Sends `OpenTerminal` and waits for `TerminalOpened`. Returns the terminal
/// metadata on success, or an error if the remote side refuses.
pub async fn open_terminal<T: Transport>(
    stream: &mut EncryptedStream<T>,
    rows: u32,
    cols: u32,
) -> Result<TerminalInfo> {
    send_action(
        stream,
        terminal_action::Union::Open(OpenTerminal {
            terminal_id: 0,
            rows,
            cols,
        }),
    )
    .await
    .context("sending OpenTerminal")?;

    // Wait for TerminalOpened.
    let resp = timeout(OPEN_TERMINAL_TIMEOUT, recv_terminal_response(stream))
        .await
        .map_err(|_| anyhow::anyhow!(
            "timed out waiting for TerminalOpened after {}s",
            OPEN_TERMINAL_TIMEOUT.as_secs()
        ))?
        .context("waiting for TerminalOpened")?;

    match resp {
        terminal_response::Union::Opened(opened) => terminal_info_from_opened(opened),
        terminal_response::Union::Error(e) => {
            bail!("terminal error on open: {}", e.message)
        }
        other => bail!("expected TerminalOpened, got {other:?}"),
    }
}

/// Compression threshold: payloads larger than this are zstd-compressed.
const COMPRESS_THRESHOLD: usize = 1024;

/// Send stdin data to the remote terminal.
///
/// Payloads exceeding [`COMPRESS_THRESHOLD`] bytes are zstd-compressed
/// (level 3) and sent with `compressed: true`.
pub async fn send_terminal_data<T: Transport>(
    stream: &mut EncryptedStream<T>,
    terminal_id: i32,
    data: &[u8],
) -> Result<()> {
    let (payload, compressed) = if data.len() > COMPRESS_THRESHOLD {
        let compressed_data =
            zstd::encode_all(data, 3).context("zstd compression failed")?;
        (compressed_data, true)
    } else {
        (data.to_vec(), false)
    };

    send_action(
        stream,
        terminal_action::Union::Data(TerminalData {
            terminal_id,
            data: payload,
            compressed,
        }),
    )
    .await
    .context("sending TerminalData")
}

/// Receive the next terminal event (stdout data, close, or error).
pub async fn recv_terminal_data<T: Transport>(
    stream: &mut EncryptedStream<T>,
) -> Result<TerminalEvent> {
    let resp = recv_terminal_response(stream)
        .await
        .context("receiving terminal data")?;

    match resp {
        terminal_response::Union::Data(td) => {
            let data = if td.compressed {
                zstd::decode_all(td.data.as_slice())
                    .context("zstd decompression of terminal data failed")?
            } else {
                td.data
            };
            Ok(TerminalEvent::Data(data))
        }
        terminal_response::Union::Closed(c) => Ok(TerminalEvent::Closed {
            exit_code: c.exit_code,
        }),
        terminal_response::Union::Error(e) => Ok(TerminalEvent::Error(e.message)),
        terminal_response::Union::Opened(_) => {
            bail!("unexpected TerminalOpened during data receive")
        }
    }
}

/// Resize the remote terminal PTY.
pub async fn resize_terminal<T: Transport>(
    stream: &mut EncryptedStream<T>,
    terminal_id: i32,
    rows: u32,
    cols: u32,
) -> Result<()> {
    send_action(
        stream,
        terminal_action::Union::Resize(ResizeTerminal {
            terminal_id,
            rows,
            cols,
        }),
    )
    .await
    .context("sending ResizeTerminal")
}

/// Close the remote terminal.
pub async fn close_terminal<T: Transport>(
    stream: &mut EncryptedStream<T>,
    terminal_id: i32,
) -> Result<()> {
    send_action(
        stream,
        terminal_action::Union::Close(CloseTerminal { terminal_id }),
    )
    .await
    .context("sending CloseTerminal")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::hbb::TerminalOpened;
    use crate::transport::FramedTransport;
    use tokio::io::duplex;

    // -- Test-only transport over tokio duplex --

    struct DuplexTransport {
        framed: FramedTransport<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn pair() -> (Self, Self) {
            let (a, b) = duplex(8192);
            (
                Self {
                    framed: FramedTransport::new(a),
                },
                Self {
                    framed: FramedTransport::new(b),
                },
            )
        }
    }

    impl Transport for DuplexTransport {
        async fn connect(_addr: &str) -> Result<Self> {
            unimplemented!("use DuplexTransport::pair()")
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

    /// Helper: send a raw Message on a DuplexTransport-backed EncryptedStream.
    async fn send_msg(stream: &mut EncryptedStream<DuplexTransport>, msg: &Message) -> Result<()> {
        let mut buf = Vec::new();
        msg.encode(&mut buf)?;
        stream.send(&buf).await
    }

    /// Helper: receive and decode a Message from an EncryptedStream.
    async fn recv_msg(stream: &mut EncryptedStream<DuplexTransport>) -> Result<Message> {
        let raw = stream.recv().await?;
        Ok(Message::decode(raw.as_slice())?)
    }

    fn terminal_response_msg(inner: terminal_response::Union) -> Message {
        Message {
            union: Some(message::Union::TerminalResponse(
                crate::proto::hbb::TerminalResponse {
                    union: Some(inner),
                },
            )),
        }
    }

    // -- Tests --

    #[tokio::test]
    async fn open_terminal_sends_open_and_receives_opened() {
        let (ct, st) = DuplexTransport::pair();
        let key = [42u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            let info = open_terminal(&mut client, 24, 80).await.unwrap();
            (info, client)
        });

        let server_task = tokio::spawn(async move {
            // Receive the OpenTerminal action.
            let msg = recv_msg(&mut server).await.unwrap();
            let action = match msg.union {
                Some(message::Union::TerminalAction(ta)) => ta.union.unwrap(),
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            match action {
                terminal_action::Union::Open(open) => {
                    assert_eq!(open.terminal_id, 0);
                    assert_eq!(open.rows, 24);
                    assert_eq!(open.cols, 80);
                }
                other => panic!("expected Open, got {other:?}"),
            }

            // Send TerminalOpened response.
            let resp = terminal_response_msg(terminal_response::Union::Opened(TerminalOpened {
                terminal_id: 1,
                success: true,
                message: String::new(),
                pid: 12345,
                service_id: "svc-001".to_string(),
                persistent_sessions: vec![],
            }));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (info, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        assert_eq!(info.terminal_id, 1);
        assert_eq!(info.pid, 12345);
        assert_eq!(info.service_id, "svc-001");
    }

    #[tokio::test]
    async fn open_terminal_returns_error_on_refusal() {
        let (ct, st) = DuplexTransport::pair();
        let key = [42u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            let result = open_terminal(&mut client, 24, 80).await;
            (result, client)
        });

        let server_task = tokio::spawn(async move {
            let _msg = recv_msg(&mut server).await.unwrap();
            let resp = terminal_response_msg(terminal_response::Union::Opened(TerminalOpened {
                terminal_id: 0,
                success: false,
                message: "terminal disabled".to_string(),
                pid: 0,
                service_id: String::new(),
                persistent_sessions: vec![],
            }));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (result, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        let err = result.unwrap_err();
        assert!(
            format!("{err}").contains("terminal disabled"),
            "error should contain refusal message: {err}"
        );
    }

    #[tokio::test]
    async fn send_and_recv_terminal_data() {
        let (ct, st) = DuplexTransport::pair();
        let key = [7u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            // Send stdin.
            send_terminal_data(&mut client, 1, b"ls -la\n").await.unwrap();
            // Receive stdout.
            let event = recv_terminal_data(&mut client).await.unwrap();
            (event, client)
        });

        let server_task = tokio::spawn(async move {
            // Receive stdin from client.
            let msg = recv_msg(&mut server).await.unwrap();
            let action = match msg.union {
                Some(message::Union::TerminalAction(ta)) => ta.union.unwrap(),
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            match action {
                terminal_action::Union::Data(td) => {
                    assert_eq!(td.terminal_id, 1);
                    assert_eq!(td.data, b"ls -la\n");
                    assert!(!td.compressed);
                }
                other => panic!("expected Data, got {other:?}"),
            }

            // Send stdout back.
            let resp = terminal_response_msg(terminal_response::Union::Data(TerminalData {
                terminal_id: 1,
                data: b"total 42\ndrwxr-xr-x  5 user user 160 Mar 14 10:00 .\n".to_vec(),
                compressed: false,
            }));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (event, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        match event {
            TerminalEvent::Data(data) => {
                assert!(data.starts_with(b"total 42"));
            }
            other => panic!("expected Data event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resize_terminal_sends_correct_message() {
        let (ct, st) = DuplexTransport::pair();
        let key = [11u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            resize_terminal(&mut client, 1, 48, 120).await.unwrap();
            client
        });

        let server_task = tokio::spawn(async move {
            let msg = recv_msg(&mut server).await.unwrap();
            let action = match msg.union {
                Some(message::Union::TerminalAction(ta)) => ta.union.unwrap(),
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            match action {
                terminal_action::Union::Resize(r) => {
                    assert_eq!(r.terminal_id, 1);
                    assert_eq!(r.rows, 48);
                    assert_eq!(r.cols, 120);
                }
                other => panic!("expected Resize, got {other:?}"),
            }
            server
        });

        let _client = client_task.await.unwrap();
        let _server = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn close_terminal_sends_correct_message() {
        let (ct, st) = DuplexTransport::pair();
        let key = [99u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            close_terminal(&mut client, 3).await.unwrap();
            client
        });

        let server_task = tokio::spawn(async move {
            let msg = recv_msg(&mut server).await.unwrap();
            let action = match msg.union {
                Some(message::Union::TerminalAction(ta)) => ta.union.unwrap(),
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            match action {
                terminal_action::Union::Close(c) => {
                    assert_eq!(c.terminal_id, 3);
                }
                other => panic!("expected Close, got {other:?}"),
            }
            server
        });

        let _client = client_task.await.unwrap();
        let _server = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn recv_terminal_data_handles_closed_event() {
        let (ct, st) = DuplexTransport::pair();
        let key = [55u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            let event = recv_terminal_data(&mut client).await.unwrap();
            (event, client)
        });

        let server_task = tokio::spawn(async move {
            let resp = terminal_response_msg(terminal_response::Union::Closed(
                crate::proto::hbb::TerminalClosed {
                    terminal_id: 1,
                    exit_code: 0,
                },
            ));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (event, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        match event {
            TerminalEvent::Closed { exit_code } => assert_eq!(exit_code, 0),
            other => panic!("expected Closed event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn recv_terminal_data_handles_error_event() {
        let (ct, st) = DuplexTransport::pair();
        let key = [88u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            let event = recv_terminal_data(&mut client).await.unwrap();
            (event, client)
        });

        let server_task = tokio::spawn(async move {
            let resp = terminal_response_msg(terminal_response::Union::Error(
                crate::proto::hbb::TerminalError {
                    terminal_id: 1,
                    message: "PTY allocation failed".to_string(),
                },
            ));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (event, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        match event {
            TerminalEvent::Error(msg) => assert_eq!(msg, "PTY allocation failed"),
            other => panic!("expected Error event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn recv_skips_non_terminal_messages() {
        let (ct, st) = DuplexTransport::pair();
        let key = [33u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            let event = recv_terminal_data(&mut client).await.unwrap();
            (event, client)
        });

        let server_task = tokio::spawn(async move {
            // Send a non-terminal message first (TestDelay).
            let noise = Message {
                union: Some(message::Union::TestDelay(crate::proto::hbb::TestDelay {
                    time: 0,
                    from_client: false,
                    last_delay: 0,
                    target_bitrate: 0,
                })),
            };
            send_msg(&mut server, &noise).await.unwrap();

            // Now send the actual terminal data.
            let resp = terminal_response_msg(terminal_response::Union::Data(TerminalData {
                terminal_id: 1,
                data: b"hello".to_vec(),
                compressed: false,
            }));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (event, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        match event {
            TerminalEvent::Data(data) => assert_eq!(data, b"hello"),
            other => panic!("expected Data, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn small_payload_is_not_compressed() {
        let (ct, st) = DuplexTransport::pair();
        let key = [44u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let small_data = b"short payload";

        let client_task = tokio::spawn(async move {
            send_terminal_data(&mut client, 1, small_data).await.unwrap();
            client
        });

        let server_task = tokio::spawn(async move {
            let msg = recv_msg(&mut server).await.unwrap();
            let action = match msg.union {
                Some(message::Union::TerminalAction(ta)) => ta.union.unwrap(),
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            match action {
                terminal_action::Union::Data(td) => {
                    assert!(!td.compressed, "small payload should not be compressed");
                    assert_eq!(td.data, small_data);
                }
                other => panic!("expected Data, got {other:?}"),
            }
            server
        });

        let _client = client_task.await.unwrap();
        let _server = server_task.await.unwrap();
    }

    #[tokio::test]
    async fn large_payload_is_compressed_and_roundtrips() {
        let (ct, st) = DuplexTransport::pair();
        let key = [77u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        // Create a payload larger than COMPRESS_THRESHOLD (1024 bytes).
        let large_data: Vec<u8> = (0..2048).map(|i| (i % 256) as u8).collect();
        let large_clone = large_data.clone();
        let server_expected_len = large_data.len();

        // Client sends large data; server verifies compressed flag and
        // sends it back as a compressed TerminalResponse so client can
        // verify decompression round-trip.
        let client_task = tokio::spawn(async move {
            send_terminal_data(&mut client, 1, &large_data).await.unwrap();
            let event = recv_terminal_data(&mut client).await.unwrap();
            (event, client)
        });

        let server_task = tokio::spawn(async move {
            let msg = recv_msg(&mut server).await.unwrap();
            let action = match msg.union {
                Some(message::Union::TerminalAction(ta)) => ta.union.unwrap(),
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            let (wire_data, was_compressed) = match action {
                terminal_action::Union::Data(td) => {
                    assert!(td.compressed, "large payload should be compressed");
                    assert!(
                        td.data.len() < server_expected_len,
                        "compressed data should be smaller: {} vs {}",
                        td.data.len(),
                        server_expected_len,
                    );
                    (td.data, td.compressed)
                }
                other => panic!("expected Data, got {other:?}"),
            };

            // Echo back as a compressed TerminalResponse.
            let resp = terminal_response_msg(terminal_response::Union::Data(TerminalData {
                terminal_id: 1,
                data: wire_data,
                compressed: was_compressed,
            }));
            send_msg(&mut server, &resp).await.unwrap();
            server
        });

        let (event, _client) = client_task.await.unwrap();
        let _server = server_task.await.unwrap();

        match event {
            TerminalEvent::Data(data) => {
                assert_eq!(data, large_clone, "decompressed data should match original");
            }
            other => panic!("expected Data event, got {other:?}"),
        }
    }
}
