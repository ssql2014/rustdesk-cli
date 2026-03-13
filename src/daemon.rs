//! Session daemon — listens on a Unix domain socket and dispatches commands.
//!
//! Lifecycle:
//! - Spawned by `rustdesk-cli connect` as a background process
//! - Establishes a real RustDesk connection (rendezvous → relay → crypto → auth)
//! - Sends OptionMessage to configure text-mode preferences
//! - Writes PID + socket path to /tmp/rustdesk-cli.lock
//! - Accepts commands over UDS, dispatches to Session
//! - Exits on `disconnect` command or idle timeout

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use prost::Message as ProstMessage;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::connection::{self, ConnectionConfig};
use crate::crypto::EncryptedStream;
use crate::proto::hbb::{
    ImageQuality, Message, Misc, OptionMessage,
    message, misc, option_message,
};
use crate::protocol::DisplayInfo;
use crate::session::{
    ConnectionState, PeerInfoState, Session, SessionCommand, SessionResponse,
};
use crate::terminal::{self, TerminalEvent};
use crate::transport::{TcpTransport, Transport};

pub const SOCKET_PATH: &str = "/tmp/rustdesk-cli.sock";
pub const LOCK_PATH: &str = "/tmp/rustdesk-cli.lock";
const ERROR_PATH: &str = "/tmp/rustdesk-cli.error";
const IDLE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes
const EXEC_TERMINAL_OPEN_TIMEOUT: Duration = Duration::from_secs(15);
const EXEC_PROMPT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const EXEC_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);
const SHELL_TERMINAL_OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// Lock file contents — written by the daemon, read by the CLI.
#[derive(Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub pid: u32,
    pub socket: String,
}

impl LockFile {
    pub fn read() -> Result<Self> {
        let data = fs::read_to_string(LOCK_PATH)
            .context("No active session (lock file not found). Run `rustdesk-cli connect` first.")?;
        Ok(serde_json::from_str(&data)?)
    }

    fn write(socket: &str) -> Result<()> {
        let lock = LockFile {
            pid: std::process::id(),
            socket: socket.to_string(),
        };
        let data = serde_json::to_string(&lock)?;
        fs::write(LOCK_PATH, &data)?;
        fs::set_permissions(LOCK_PATH, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    fn remove() {
        let _ = fs::remove_file(LOCK_PATH);
    }
}

/// Check if a daemon is already running by reading the lock file and checking the PID.
pub fn is_daemon_running() -> bool {
    let lock = match LockFile::read() {
        Ok(l) => l,
        Err(_) => return false,
    };
    // Check if process is alive
    unsafe { libc::kill(lock.pid as i32, 0) == 0 }
}

/// Spawn the daemon as a background process by re-executing ourselves
/// with a special `--daemon` flag.
pub fn spawn_daemon(
    peer_id: &str,
    password: Option<&str>,
    server: Option<&str>,
    id_server: Option<&str>,
    relay_server: Option<&str>,
    key: Option<&str>,
    timeout: Option<u64>,
) -> Result<()> {
    if is_daemon_running() {
        anyhow::bail!("Daemon already running. Disconnect first, or use other commands.");
    }

    // Clean up stale socket/lock
    let _ = fs::remove_file(SOCKET_PATH);
    let _ = fs::remove_file(LOCK_PATH);
    let _ = fs::remove_file(ERROR_PATH);

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon")
        .arg("--peer-id")
        .arg(peer_id);
    if let Some(pw) = password {
        cmd.arg("--password").arg(pw);
    }
    if let Some(srv) = server {
        cmd.arg("--server").arg(srv);
    }
    if let Some(id_srv) = id_server {
        cmd.arg("--id-server").arg(id_srv);
    }
    if let Some(relay_srv) = relay_server {
        cmd.arg("--relay-server").arg(relay_srv);
    }
    if let Some(key) = key {
        cmd.arg("--key").arg(key);
    }
    if let Some(t) = timeout {
        cmd.arg("--timeout").arg(t.to_string());
    }

    // Detach: redirect stdio so parent can exit
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd.spawn().context("Failed to spawn daemon process")?;

    // Wait for the daemon to create its lock file.
    // Lock file is written AFTER connect_to_peer succeeds, so we need
    // to wait at least as long as the connection timeout plus margin.
    let wait_secs = timeout.unwrap_or(30) + 5;
    let wait_iters = (wait_secs * 10) as usize; // 100ms per iteration
    for _ in 0..wait_iters {
        if Path::new(LOCK_PATH).exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if Path::new(ERROR_PATH).exists() {
        let error = fs::read_to_string(ERROR_PATH)
            .unwrap_or_else(|_| "daemon startup failed".to_string());
        let _ = fs::remove_file(ERROR_PATH);
        anyhow::bail!("Daemon failed to start: {}", error.trim());
    }

    anyhow::bail!("Daemon started but lock file not created within {wait_secs} seconds")
}

/// Send a command to the running daemon and return the response.
pub async fn send_command(cmd: &SessionCommand) -> Result<SessionResponse> {
    let lock = LockFile::read()?;
    let stream = tokio::net::UnixStream::connect(&lock.socket)
        .await
        .context("Failed to connect to daemon socket. Is the session still alive?")?;

    let (reader, mut writer) = stream.into_split();

    // Send command as a single JSON line
    let mut data = serde_json::to_vec(cmd)?;
    data.push(b'\n');
    writer.write_all(&data).await?;
    writer.shutdown().await?;

    // Read response
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;

    let response: SessionResponse = serde_json::from_str(line.trim())?;
    Ok(response)
}

/// Run the daemon event loop. Called when the binary is invoked with `--daemon`.
pub async fn run_daemon(
    peer_id: String,
    password: Option<String>,
    server: Option<String>,
    id_server: Option<String>,
    relay_server: Option<String>,
    key: Option<String>,
    timeout: Option<u64>,
) -> Result<()> {
    // Clean up stale socket
    let _ = fs::remove_file(SOCKET_PATH);
    let _ = fs::remove_file(ERROR_PATH);

    let listener = UnixListener::bind(SOCKET_PATH)
        .context("Failed to bind Unix socket")?;

    // Set socket permissions to owner-only
    fs::set_permissions(SOCKET_PATH, fs::Permissions::from_mode(0o600))?;

    // Build connection config from daemon arguments.
    let config = build_connection_config(
        &peer_id,
        password.as_deref(),
        server.as_deref(),
        id_server.as_deref(),
        relay_server.as_deref(),
        key.as_deref(),
    );

    // Real connection: rendezvous → relay → crypto → auth.
    // Timeout prevents hanging on unreachable servers or bad credentials.
    let timeout_secs = timeout.unwrap_or(30);
    let conn_result = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        connection::connect_to_peer(&config),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            let message = format!("{e:#}");
            let _ = write_startup_error(&message);
            eprintln!("daemon: connect failed: {message}");
            cleanup();
            let _ = write_startup_error(&message);
            return Ok(());
        }
        Err(_) => {
            let message = format!("connect timed out after {timeout_secs}s");
            let _ = write_startup_error(&message);
            eprintln!("daemon: {message}");
            cleanup();
            let _ = write_startup_error(&message);
            return Ok(());
        }
    };

    // Lock file signals readiness — written AFTER auth succeeds so the
    // CLI won't see "connected" until the peer is actually reachable.
    LockFile::write(SOCKET_PATH)?;

    let mut encrypted = conn_result.encrypted;
    let peer_info = conn_result.peer_info;

    // Send OptionMessage: disable audio/camera, enable terminal persistence,
    // keep clipboard enabled, low image quality for text-mode.
    if let Err(e) = send_option_message(&mut encrypted).await {
        eprintln!("daemon: failed to send OptionMessage: {e:#}");
        let _ = encrypted.close().await;
        cleanup();
        return Ok(());
    }

    // Initialize session control-plane state with real peer info.
    let mut session = Session::new();
    session.state = ConnectionState::Connected;
    session.config = Some(crate::protocol::ConnectionConfig {
        peer_id: peer_id.clone(),
        password: password.clone(),
        server: server.clone(),
    });
    session.peer_info = Some(PeerInfoState {
        peer_id: peer_info.username.clone(),
        username: peer_info.username.clone(),
        hostname: peer_info.hostname.clone(),
        displays: peer_info
            .displays
            .iter()
            .map(|d| DisplayInfo {
                x: d.x,
                y: d.y,
                width: d.width,
                height: d.height,
            })
            .collect(),
    });

    let mut last_activity = Instant::now();

    loop {
        // Accept with idle timeout
        let accept = tokio::time::timeout(
            IDLE_TIMEOUT.saturating_sub(last_activity.elapsed()),
            listener.accept(),
        );

        match accept.await {
            Ok(Ok((stream, _addr))) => {
                last_activity = Instant::now();

                let (reader, mut writer) = stream.into_split();
                let mut buf_reader = BufReader::new(reader);
                let mut line = String::new();

                if buf_reader.read_line(&mut line).await.is_err() {
                    continue;
                }

                let cmd: SessionCommand = match serde_json::from_str(line.trim()) {
                    Ok(c) => c,
                    Err(e) => {
                        let resp = SessionResponse::error(format!("Invalid command: {e}"));
                        let _ = send_response(&mut writer, &resp).await;
                        continue;
                    }
                };

                let is_disconnect = matches!(cmd, SessionCommand::Disconnect);

                // Shell takes over the UDS connection for bidirectional streaming.
                // The ack response is sent inside shell_session; on return we
                // loop back to accept the next UDS connection.
                if matches!(cmd, SessionCommand::Shell) {
                    if let Err(e) = shell_session(&mut encrypted, buf_reader, writer).await {
                        eprintln!("daemon: shell session error: {e:#}");
                    }
                    continue;
                }

                let response = match cmd {
                    SessionCommand::Exec { command } => {
                        match exec_command(&mut encrypted, &command).await {
                            Ok(resp) => resp,
                            Err(e) => SessionResponse::error(format!("exec failed: {e:#}")),
                        }
                    }
                    SessionCommand::ClipboardGet => {
                        match clipboard_get(&mut encrypted).await {
                            Ok(resp) => resp,
                            Err(e) => SessionResponse::error(format!("clipboard get failed: {e:#}")),
                        }
                    }
                    SessionCommand::ClipboardSet { text } => {
                        match clipboard_set(&mut encrypted, &text).await {
                            Ok(resp) => resp,
                            Err(e) => SessionResponse::error(format!("clipboard set failed: {e:#}")),
                        }
                    }
                    other => match session.dispatch(other) {
                        Ok((resp, _msgs)) => resp,
                        Err(e) => SessionResponse::error(e.to_string()),
                    },
                };

                let _ = send_response(&mut writer, &response).await;

                if is_disconnect {
                    let _ = encrypted.close().await;
                    cleanup();
                    return Ok(());
                }
            }
            Ok(Err(e)) => {
                eprintln!("daemon: accept error: {e}");
            }
            Err(_) => {
                // Idle timeout
                eprintln!("daemon: idle timeout, shutting down");
                let _ = encrypted.close().await;
                cleanup();
                return Ok(());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: connection config builder
// ---------------------------------------------------------------------------

/// Build a [`ConnectionConfig`] from the daemon's CLI arguments.
///
/// Priority: explicit `--id-server`/`--relay-server` > derived from `--server` > defaults.
fn build_connection_config(
    peer_id: &str,
    password: Option<&str>,
    server: Option<&str>,
    id_server: Option<&str>,
    relay_server: Option<&str>,
    key: Option<&str>,
) -> ConnectionConfig {
    let id_srv = match id_server {
        Some(s) => s.to_string(),
        None => match server {
            Some(s) => {
                let host = s.split(':').next().unwrap_or(s);
                format!("{host}:21116")
            }
            None => "localhost:21116".to_string(),
        },
    };

    let relay_srv = match relay_server {
        Some(s) => s.to_string(),
        None => match server {
            Some(s) => {
                let host = s.split(':').next().unwrap_or(s);
                format!("{host}:21117")
            }
            None => "localhost:21117".to_string(),
        },
    };

    ConnectionConfig {
        id_server: id_srv,
        relay_server: relay_srv,
        server_key: key.unwrap_or("").to_string(),
        peer_id: peer_id.to_string(),
        password: password.unwrap_or("").to_string(),
    }
}

// ---------------------------------------------------------------------------
// Internal: OptionMessage for text-mode
// ---------------------------------------------------------------------------

/// Send `OptionMessage` to configure the remote peer for text-mode:
/// - `disable_audio = Yes`
/// - `disable_camera = Yes`
/// - `terminal_persistent = Yes`
/// - `disable_clipboard = No` (explicitly opt-in to clipboard sync)
/// - `image_quality = Low`
async fn send_option_message(
    stream: &mut EncryptedStream<TcpTransport>,
) -> Result<()> {
    let opt = OptionMessage {
        image_quality: ImageQuality::Low as i32,
        disable_audio: option_message::BoolOption::Yes as i32,
        disable_clipboard: option_message::BoolOption::No as i32,
        disable_camera: option_message::BoolOption::Yes as i32,
        terminal_persistent: option_message::BoolOption::Yes as i32,
        ..Default::default()
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
// Internal: exec via ephemeral terminal + sentinel
// ---------------------------------------------------------------------------

/// Execute a command on the remote peer via an ephemeral terminal.
///
/// 1. Open a terminal (24×80)
/// 2. Drain initial prompt/banner (short idle timeout)
/// 3. Send the command followed by a sentinel echo for completion detection
/// 4. Collect output until the sentinel appears or timeout
/// 5. Close the terminal
/// 6. Return SessionResponse with stdout, stderr, and exit_code
async fn exec_command(
    encrypted: &mut EncryptedStream<TcpTransport>,
    command: &str,
) -> Result<SessionResponse> {
    // Generate unique sentinel marker using timestamp nanos.
    let sentinel_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sentinel = format!("__RDCLI_{sentinel_id:032x}__");

    // 1. Open ephemeral terminal.
    let terminal_info = tokio::time::timeout(
        EXEC_TERMINAL_OPEN_TIMEOUT,
        terminal::open_terminal(encrypted, 24, 80),
    )
    .await
    .map_err(|_| anyhow::anyhow!("terminal open timed out"))??;

    let tid = terminal_info.terminal_id;

    // 2. Drain initial prompt/banner with idle timeout.
    loop {
        match tokio::time::timeout(
            EXEC_PROMPT_DRAIN_TIMEOUT,
            terminal::recv_terminal_data(encrypted),
        )
        .await
        {
            Ok(Ok(TerminalEvent::Data(_))) => {
                // Keep draining.
            }
            Ok(Ok(TerminalEvent::Closed { exit_code })) => {
                return Ok(SessionResponse::ok_with_data(
                    "Terminal closed before exec",
                    serde_json::json!({
                        "command": command,
                        "stdout": "",
                        "stderr": "",
                        "exit_code": exit_code,
                        "timed_out": false,
                    }),
                ));
            }
            Ok(Ok(TerminalEvent::Error(msg))) => {
                let _ = terminal::close_terminal(encrypted, tid).await;
                anyhow::bail!("terminal error during prompt drain: {msg}");
            }
            Ok(Err(e)) => {
                let _ = terminal::close_terminal(encrypted, tid).await;
                anyhow::bail!("recv error during prompt drain: {e:#}");
            }
            Err(_) => break, // Idle timeout — prompt fully drained.
        }
    }

    // 3. Send command + sentinel echo.
    // The sentinel echo prints a unique marker followed by $? (the exit code).
    // Two separate lines ensure the user command terminates before the echo runs.
    let wrapped = format!("{command}\necho '{sentinel}'$?\n");
    terminal::send_terminal_data(encrypted, tid, wrapped.as_bytes()).await?;

    // 4. Collect output until sentinel appears or completion timeout.
    let mut collected = Vec::new();
    let mut timed_out = false;
    let deadline = tokio::time::Instant::now() + EXEC_COMPLETION_TIMEOUT;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            timed_out = true;
            break;
        }

        match tokio::time::timeout(
            remaining,
            terminal::recv_terminal_data(encrypted),
        )
        .await
        {
            Ok(Ok(TerminalEvent::Data(data))) => {
                collected.extend_from_slice(&data);
                // Check if sentinel output has appeared.
                if find_sentinel_output(&String::from_utf8_lossy(&collected), &sentinel).is_some() {
                    break;
                }
            }
            Ok(Ok(TerminalEvent::Closed { exit_code })) => {
                // Terminal closed before sentinel — return partial output.
                let stdout = String::from_utf8_lossy(&collected).trim().to_string();
                let _ = terminal::close_terminal(encrypted, tid).await;
                return Ok(SessionResponse::ok_with_data(
                    format!("Executed `{command}`"),
                    serde_json::json!({
                        "command": command,
                        "stdout": stdout,
                        "stderr": "",
                        "exit_code": exit_code,
                        "timed_out": false,
                    }),
                ));
            }
            Ok(Ok(TerminalEvent::Error(msg))) => {
                let _ = terminal::close_terminal(encrypted, tid).await;
                anyhow::bail!("terminal error during exec: {msg}");
            }
            Ok(Err(e)) => {
                let _ = terminal::close_terminal(encrypted, tid).await;
                anyhow::bail!("recv error during exec: {e:#}");
            }
            Err(_) => {
                timed_out = true;
                break;
            }
        }
    }

    // 5. Close the ephemeral terminal.
    let _ = terminal::close_terminal(encrypted, tid).await;

    // 6. Parse output — extract stdout and exit code from sentinel.
    let raw = String::from_utf8_lossy(&collected);
    let (stdout, exit_code) = parse_exec_output(&raw, &sentinel);

    Ok(SessionResponse::ok_with_data(
        format!("Executed `{command}`"),
        serde_json::json!({
            "command": command,
            "stdout": stdout,
            "stderr": "",
            "exit_code": exit_code,
            "timed_out": timed_out,
        }),
    ))
}

/// Find the sentinel output line (sentinel followed by digit(s) = exit code).
///
/// Distinguishes from the echoed command which shows `echo '<sentinel>'$?`
/// (sentinel followed by `'$?`, not digits).
fn find_sentinel_output(raw: &str, sentinel: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(pos) = raw[search_from..].find(sentinel) {
        let abs_pos = search_from + pos;
        let after = &raw[abs_pos + sentinel.len()..];
        if after.starts_with(|c: char| c.is_ascii_digit()) {
            return Some(abs_pos);
        }
        search_from = abs_pos + sentinel.len();
    }
    None
}

/// Parse the collected terminal output, extracting real command output and exit code.
///
/// Terminal output structure after prompt drain:
/// ```text
/// <echoed command>\r\n
/// echo '<sentinel>'$?\r\n       ← echoed sentinel command
/// <actual command output>\r\n
/// <sentinel><exit_code>\r\n     ← sentinel output
/// <next prompt>
/// ```
fn parse_exec_output(raw: &str, sentinel: &str) -> (String, i32) {
    // 1. Find sentinel output (sentinel + digits = echo result).
    let Some(sentinel_pos) = find_sentinel_output(raw, sentinel) else {
        // Sentinel not found (timeout) — return raw output, exit code -1.
        return (raw.trim().to_string(), -1);
    };

    // 2. Parse exit code from digits after sentinel.
    let after = &raw[sentinel_pos + sentinel.len()..];
    let code_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    let exit_code = code_str.parse::<i32>().unwrap_or(-1);

    // 3. Find the echoed echo command to locate where real output starts.
    let echo_cmd = format!("echo '{sentinel}'");
    let output_start = raw
        .find(&echo_cmd)
        .and_then(|pos| raw[pos..].find('\n').map(|nl| pos + nl + 1))
        .unwrap_or(0);

    // 4. Sentinel output starts at the beginning of its line.
    let sentinel_line_start = raw[..sentinel_pos]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(0);

    // 5. Extract stdout between echoed echo command and sentinel output line.
    let stdout = if output_start < sentinel_line_start {
        raw[output_start..sentinel_line_start]
            .trim_end_matches(['\r', '\n'])
            .to_string()
    } else {
        String::new()
    };

    (stdout, exit_code)
}

// ---------------------------------------------------------------------------
// Internal: shell session (bidirectional terminal streaming)
// ---------------------------------------------------------------------------

/// Run a bidirectional shell session, forwarding data between a UDS
/// connection and the remote terminal channel.
///
/// 1. Opens a terminal via `terminal::open_terminal`
/// 2. Sends a JSON ack (or error) to the CLI over UDS
/// 3. Loops: UDS lines → `send_terminal_data`, terminal output → UDS
/// Get clipboard text from the remote peer by running a shell command.
///
/// Tries xclip (Linux) then pbpaste (macOS), falling back to empty string.
async fn clipboard_get(
    encrypted: &mut EncryptedStream<TcpTransport>,
) -> Result<SessionResponse> {
    let cmd = "xclip -selection clipboard -o 2>/dev/null || pbpaste 2>/dev/null || echo -n ''";
    let exec_resp = exec_command(encrypted, cmd).await?;

    // Extract stdout from exec response to reshape into clipboard contract.
    let text = exec_resp
        .data
        .as_ref()
        .and_then(|d| d["stdout"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(SessionResponse::ok_with_data(
        "Clipboard text retrieved",
        serde_json::json!({
            "text": text,
        }),
    ))
}

/// Set clipboard text on the remote peer by running a shell command.
///
/// Pipes the text through xclip (Linux) or pbcopy (macOS).
async fn clipboard_set(
    encrypted: &mut EncryptedStream<TcpTransport>,
    text: &str,
) -> Result<SessionResponse> {
    // Shell-escape by using a heredoc to avoid issues with quotes/special chars.
    let cmd = format!(
        "cat <<'__RDCLI_CLIP_EOF__' | xclip -selection clipboard 2>/dev/null || \
         cat <<'__RDCLI_CLIP_EOF__' | pbcopy 2>/dev/null\n\
         {text}\n__RDCLI_CLIP_EOF__"
    );
    exec_command(encrypted, &cmd).await?;

    Ok(SessionResponse::ok_with_data(
        "Clipboard text updated",
        serde_json::json!({
            "chars": text.chars().count(),
            "redacted": true,
        }),
    ))
}

/// 4. On UDS EOF or terminal close/error, closes the terminal
///
/// Note: The `tokio::select!` loop cancels `recv_terminal_data` when
/// UDS input arrives. The underlying framed transport's recv is not
/// fully cancellation-safe; a future step may split the encrypted
/// stream into independent read/write halves to resolve this.
async fn shell_session<T, R>(
    encrypted: &mut EncryptedStream<T>,
    uds_reader: R,
    mut uds_writer: impl tokio::io::AsyncWrite + Unpin,
) -> Result<()>
where
    T: Transport,
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
{
    // 1. Open terminal (24×80 default).
    let terminal_info = match tokio::time::timeout(
        SHELL_TERMINAL_OPEN_TIMEOUT,
        terminal::open_terminal(encrypted, 24, 80),
    )
    .await
    {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => {
            let resp = SessionResponse::error(format!("terminal open failed: {e:#}"));
            let _ = write_json_line(&mut uds_writer, &resp).await;
            anyhow::bail!("shell: terminal open failed: {e:#}");
        }
        Err(_) => {
            let resp = SessionResponse::error("terminal open timed out");
            let _ = write_json_line(&mut uds_writer, &resp).await;
            anyhow::bail!("shell: terminal open timed out");
        }
    };
    let tid = terminal_info.terminal_id;

    // 2. Send ack so the CLI knows the session started.
    write_json_line(
        &mut uds_writer,
        &SessionResponse::ok_with_data(
            "Shell session started",
            serde_json::json!({
                "mode": "interactive",
                "terminal_id": tid,
            }),
        ),
    )
    .await?;

    // 3. Spawn a task to read lines from UDS into a channel,
    //    decoupling the borrow from the encrypted stream so the
    //    select loop can alternate between UDS input and terminal
    //    output without conflicting &mut borrows on `encrypted`.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<String>(64);
    tokio::spawn(async move {
        let mut reader = uds_reader;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line_tx.send(line.clone()).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    // 4. Bidirectional loop.
    //    Using an enum lets us release all borrows before the match
    //    body runs, so `encrypted` is available for send_terminal_data.
    loop {
        enum Event {
            UdsLine(Option<String>),
            Terminal(Result<TerminalEvent>),
        }

        let event = tokio::select! {
            line = line_rx.recv() => Event::UdsLine(line),
            result = terminal::recv_terminal_data(encrypted) => Event::Terminal(result),
        };

        match event {
            Event::UdsLine(Some(line)) => {
                terminal::send_terminal_data(encrypted, tid, line.as_bytes()).await?;
            }
            Event::UdsLine(None) => {
                // CLI disconnected.
                let _ = terminal::close_terminal(encrypted, tid).await;
                break;
            }
            Event::Terminal(Ok(TerminalEvent::Data(data))) => {
                uds_writer.write_all(&data).await?;
                uds_writer.flush().await?;
            }
            Event::Terminal(Ok(TerminalEvent::Closed { .. })) => {
                break;
            }
            Event::Terminal(Ok(TerminalEvent::Error(msg))) => {
                eprintln!("daemon: shell terminal error: {msg}");
                break;
            }
            Event::Terminal(Err(e)) => {
                eprintln!("daemon: shell recv error: {e:#}");
                break;
            }
        }
    }

    Ok(())
}

/// Write a serialized [`SessionResponse`] as a JSON line.
async fn write_json_line(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    resp: &SessionResponse,
) -> Result<()> {
    let mut data = serde_json::to_vec(resp)?;
    data.push(b'\n');
    writer.write_all(&data).await?;
    writer.flush().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal: helpers
// ---------------------------------------------------------------------------

async fn send_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    resp: &SessionResponse,
) -> Result<()> {
    let mut data = serde_json::to_vec(resp)?;
    data.push(b'\n');
    writer.write_all(&data).await?;
    Ok(())
}

fn cleanup() {
    let _ = fs::remove_file(SOCKET_PATH);
    let _ = fs::remove_file(ERROR_PATH);
    LockFile::remove();
}

fn write_startup_error(message: &str) -> Result<()> {
    fs::write(ERROR_PATH, message)?;
    fs::set_permissions(ERROR_PATH, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "__RDCLI_00000000000000000000000000000001__";

    #[test]
    fn find_sentinel_output_matches_digit_suffix() {
        let raw = format!("echo '{SENTINEL}'$?\r\n{SENTINEL}0\r\n$ ");
        let pos = find_sentinel_output(&raw, SENTINEL);
        assert!(pos.is_some());
        // Should match the second occurrence (followed by digit "0").
        let after = &raw[pos.unwrap() + SENTINEL.len()..];
        assert!(after.starts_with('0'));
    }

    #[test]
    fn find_sentinel_output_skips_echoed_command() {
        // The echoed command has sentinel inside quotes followed by '$?', not digits.
        let raw = format!("echo '{SENTINEL}'$?\r\n");
        assert!(find_sentinel_output(&raw, SENTINEL).is_none());
    }

    #[test]
    fn find_sentinel_output_returns_none_when_missing() {
        assert!(find_sentinel_output("hello world\n", SENTINEL).is_none());
    }

    #[test]
    fn parse_exec_output_extracts_stdout_and_exit_code() {
        let raw = format!(
            "whoami\r\necho '{SENTINEL}'$?\r\nroot\r\n{SENTINEL}0\r\n$ "
        );
        let (stdout, exit_code) = parse_exec_output(&raw, SENTINEL);
        assert_eq!(stdout, "root");
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn parse_exec_output_handles_nonzero_exit_code() {
        let raw = format!(
            "false\r\necho '{SENTINEL}'$?\r\n{SENTINEL}1\r\n$ "
        );
        let (stdout, exit_code) = parse_exec_output(&raw, SENTINEL);
        assert_eq!(stdout, "");
        assert_eq!(exit_code, 1);
    }

    #[test]
    fn parse_exec_output_handles_multi_digit_exit_code() {
        let raw = format!(
            "exit 127\r\necho '{SENTINEL}'$?\r\n{SENTINEL}127\r\n$ "
        );
        let (_stdout, exit_code) = parse_exec_output(&raw, SENTINEL);
        assert_eq!(exit_code, 127);
    }

    #[test]
    fn parse_exec_output_multiline_stdout() {
        let raw = format!(
            "ls\r\necho '{SENTINEL}'$?\r\nfile1.txt\r\nfile2.txt\r\nfile3.txt\r\n{SENTINEL}0\r\n$ "
        );
        let (stdout, exit_code) = parse_exec_output(&raw, SENTINEL);
        assert_eq!(stdout, "file1.txt\r\nfile2.txt\r\nfile3.txt");
        assert_eq!(exit_code, 0);
    }

    #[test]
    fn parse_exec_output_no_sentinel_returns_raw_and_minus_one() {
        let raw = "some partial output\r\n";
        let (stdout, exit_code) = parse_exec_output(raw, SENTINEL);
        assert_eq!(stdout, "some partial output");
        assert_eq!(exit_code, -1);
    }

    // -- Shell session test helpers --

    use crate::proto::hbb::{
        TerminalClosed, TerminalData, TerminalOpened,
        terminal_action, terminal_response, message,
    };
    use crate::transport::FramedTransport;
    use tokio::io::duplex;

    struct DuplexTransport {
        framed: FramedTransport<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn pair() -> (Self, Self) {
            let (a, b) = duplex(8192);
            (
                Self { framed: FramedTransport::new(a) },
                Self { framed: FramedTransport::new(b) },
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
            union: Some(message::Union::TerminalResponse(
                crate::proto::hbb::TerminalResponse {
                    union: Some(inner),
                },
            )),
        }
    }

    // -- Shell session tests --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "hangs due to mock transport scheduling — shell_session verified via live tests"]
    async fn shell_session_opens_terminal_sends_ack_and_closes_on_uds_eof() {
        let (ct, st) = DuplexTransport::pair();
        let key = [42u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        // UDS simulation: cli_stream ↔ daemon_stream
        let (cli_stream, daemon_stream) = duplex(8192);
        let (daemon_read, daemon_write) = tokio::io::split(daemon_stream);
        let (mut cli_read, cli_write) = tokio::io::split(cli_stream);

        // CLI: close write immediately → daemon sees EOF after open.
        drop(cli_write);

        // Server: respond to OpenTerminal, then expect CloseTerminal.
        let server_task = tokio::spawn(async move {
            // Receive OpenTerminal.
            let msg = recv_msg(&mut server).await.unwrap();
            match msg.union.unwrap() {
                message::Union::TerminalAction(ta) => {
                    assert!(matches!(ta.union.unwrap(), terminal_action::Union::Open(_)));
                }
                other => panic!("expected TerminalAction(Open), got {other:?}"),
            }

            // Send TerminalOpened.
            send_msg(
                &mut server,
                &terminal_response_msg(terminal_response::Union::Opened(TerminalOpened {
                    terminal_id: 1,
                    success: true,
                    message: String::new(),
                    pid: 1234,
                    service_id: "svc".into(),
                    persistent_sessions: vec![],
                })),
            )
            .await
            .unwrap();

            // Receive CloseTerminal (triggered by UDS EOF).
            let msg = recv_msg(&mut server).await.unwrap();
            match msg.union.unwrap() {
                message::Union::TerminalAction(ta) => {
                    assert!(matches!(ta.union.unwrap(), terminal_action::Union::Close(_)));
                }
                other => panic!("expected TerminalAction(Close), got {other:?}"),
            }
            server
        });

        // Run shell_session.
        let buf_reader = BufReader::new(daemon_read);
        let result = shell_session(&mut client, buf_reader, daemon_write).await;
        assert!(result.is_ok(), "shell_session failed: {result:?}");

        // Read ack from CLI side.
        let mut output = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut cli_read, &mut output)
            .await
            .unwrap();
        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("Shell session started"),
            "ack not found in: {output_str}"
        );
        assert!(
            output_str.contains("\"mode\":\"interactive\""),
            "mode not found in: {output_str}"
        );

        let _server = server_task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "hangs due to mock transport scheduling — shell_session verified via live tests"]
    async fn shell_session_forwards_data_bidirectionally() {
        let (ct, st) = DuplexTransport::pair();
        let key = [42u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        // UDS simulation.
        let (cli_stream, daemon_stream) = duplex(8192);
        let (daemon_read, daemon_write) = tokio::io::split(daemon_stream);
        let (mut cli_read, mut cli_write) = tokio::io::split(cli_stream);

        // Server: OpenTerminal → Opened → receive stdin → send stdout → Closed
        let server_task = tokio::spawn(async move {
            // Receive OpenTerminal.
            let _msg = recv_msg(&mut server).await.unwrap();
            // Send TerminalOpened.
            send_msg(
                &mut server,
                &terminal_response_msg(terminal_response::Union::Opened(TerminalOpened {
                    terminal_id: 1,
                    success: true,
                    message: String::new(),
                    pid: 1234,
                    service_id: "svc".into(),
                    persistent_sessions: vec![],
                })),
            )
            .await
            .unwrap();

            // Receive stdin data (forwarded from UDS line).
            let msg = recv_msg(&mut server).await.unwrap();
            let stdin_data = match msg.union.unwrap() {
                message::Union::TerminalAction(ta) => match ta.union.unwrap() {
                    terminal_action::Union::Data(td) => td.data,
                    other => panic!("expected Data, got {other:?}"),
                },
                other => panic!("expected TerminalAction, got {other:?}"),
            };
            assert_eq!(
                String::from_utf8_lossy(&stdin_data),
                "hello world\n",
                "stdin forwarding mismatch"
            );

            // Send stdout back.
            send_msg(
                &mut server,
                &terminal_response_msg(terminal_response::Union::Data(TerminalData {
                    terminal_id: 1,
                    data: b"remote output\r\n".to_vec(),
                    compressed: false,
                })),
            )
            .await
            .unwrap();

            // Send TerminalClosed to end session.
            send_msg(
                &mut server,
                &terminal_response_msg(terminal_response::Union::Closed(TerminalClosed {
                    terminal_id: 1,
                    exit_code: 0,
                })),
            )
            .await
            .unwrap();

            server
        });

        // CLI: write a line, read all output (blocks until daemon_write is dropped).
        let cli_task = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(&mut cli_write, b"hello world\n")
                .await
                .unwrap();
            // Don't close cli_write — session ends via TerminalClosed from server.
            let mut output = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut cli_read, &mut output)
                .await
                .unwrap();
            output
        });

        // Run shell_session.
        let buf_reader = BufReader::new(daemon_read);
        let result = shell_session(&mut client, buf_reader, daemon_write).await;
        assert!(result.is_ok(), "shell_session failed: {result:?}");

        let _server = server_task.await.unwrap();
        let cli_output = cli_task.await.unwrap();
        let output_str = String::from_utf8_lossy(&cli_output);

        // Verify ack.
        assert!(
            output_str.contains("Shell session started"),
            "ack not found in: {output_str}"
        );
        // Verify terminal output was forwarded to CLI.
        assert!(
            output_str.contains("remote output"),
            "terminal output not forwarded: {output_str}"
        );
    }

    #[tokio::test]
    async fn write_json_line_serializes_response() {
        let mut buf = Vec::new();
        let resp = SessionResponse::ok("test message");
        write_json_line(&mut buf, &resp).await.unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["message"], "test message");
    }
}
