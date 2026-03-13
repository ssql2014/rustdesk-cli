//! Session daemon — listens on a Unix domain socket and dispatches commands.
//!
//! Lifecycle:
//! - Spawned by `rustdesk-cli connect` as a background process
//! - Writes PID + socket path to /tmp/rustdesk-cli.lock
//! - Accepts commands over UDS, dispatches to Session
//! - Exits on `disconnect` command or idle timeout

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::session::{Session, SessionCommand, SessionResponse};

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
    _id_server: Option<String>,
    _relay_server: Option<String>,
    _key: Option<String>,
) -> Result<()> {
    // Clean up stale socket
    let _ = fs::remove_file(SOCKET_PATH);

    let listener = UnixListener::bind(SOCKET_PATH)
        .context("Failed to bind Unix socket")?;

    // Set socket permissions to owner-only
    fs::set_permissions(SOCKET_PATH, fs::Permissions::from_mode(0o600))?;

    // Write lock file
    LockFile::write(SOCKET_PATH)?;

    // Initialize session
    let mut session = Session::new();

    // Auto-connect on startup
    let connect_cmd = SessionCommand::Connect {
        peer_id,
        password,
        server,
    };
    match session.dispatch(connect_cmd) {
        Ok((resp, _msgs)) => {
            if !resp.success {
                eprintln!("daemon: connect failed: {:?}", resp.message);
                cleanup();
                return Ok(());
            }
        }
        Err(e) => {
            eprintln!("daemon: connect error: {e}");
            cleanup();
            return Ok(());
        }
    }

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
                    Ok((resp, _msgs)) => {
                        // TODO: Send protocol messages over the RustDesk connection
                        resp
                    }
                    Err(e) => SessionResponse::error(e.to_string()),
                };

                let _ = send_response(&mut writer, &response).await;

                if is_disconnect {
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
                cleanup();
                return Ok(());
            }
        }
    }
}

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
