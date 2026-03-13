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
use crate::transport::TcpTransport;

pub const SOCKET_PATH: &str = "/tmp/rustdesk-cli.sock";
pub const LOCK_PATH: &str = "/tmp/rustdesk-cli.lock";
const IDLE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

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
) -> Result<()> {
    if is_daemon_running() {
        anyhow::bail!("Daemon already running. Disconnect first, or use other commands.");
    }

    // Clean up stale socket/lock
    let _ = fs::remove_file(SOCKET_PATH);
    let _ = fs::remove_file(LOCK_PATH);

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

    // Detach: redirect stdio so parent can exit
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd.spawn().context("Failed to spawn daemon process")?;

    // Wait briefly for the daemon to create its lock file
    for _ in 0..50 {
        if Path::new(LOCK_PATH).exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    anyhow::bail!("Daemon started but lock file not created within 5 seconds")
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
) -> Result<()> {
    // Clean up stale socket
    let _ = fs::remove_file(SOCKET_PATH);

    let listener = UnixListener::bind(SOCKET_PATH)
        .context("Failed to bind Unix socket")?;

    // Set socket permissions to owner-only
    fs::set_permissions(SOCKET_PATH, fs::Permissions::from_mode(0o600))?;

    // Write lock file
    LockFile::write(SOCKET_PATH)?;

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
    let conn_result = match connection::connect_to_peer(&config).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("daemon: connect failed: {e:#}");
            cleanup();
            return Ok(());
        }
    };

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

                let response = match session.dispatch(cmd) {
                    Ok((resp, _msgs)) => resp,
                    Err(e) => SessionResponse::error(e.to_string()),
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
    LockFile::remove();
}
