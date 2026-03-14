#[allow(dead_code)]
mod connection;
#[allow(dead_code)]
mod capture;
#[allow(dead_code)]
mod crypto;
#[allow(dead_code)]
mod daemon;
#[allow(dead_code)]
mod file_transfer;
#[allow(dead_code)]
mod proto;
#[allow(dead_code)]
mod protocol;
mod permissions;
#[allow(dead_code)]
mod rendezvous;
#[allow(dead_code)]
mod terminal;
#[allow(dead_code)]
mod text_session;
#[allow(dead_code)]
mod transport;
mod version;
mod session;

use std::{
    io::{BufRead, BufReader, Write},
    net::Shutdown,
    os::unix::net::UnixStream as StdUnixStream,
    path::Path,
    process,
    str::FromStr,
};

use clap::{Parser, Subcommand, ValueEnum, error::ErrorKind};
use serde_json::{Value, json};

use crate::permissions::PermissionManager;
use crate::session::{SessionCommand, SessionResponse};

const EXIT_SUCCESS: i32 = 0;
const EXIT_CONNECTION: i32 = 1;
const EXIT_SESSION: i32 = 2;
const EXIT_INPUT: i32 = 3;
const EXIT_PERMISSION: i32 = 4;

const DEFAULT_WIDTH: i32 = 1920;
const DEFAULT_HEIGHT: i32 = 1080;

#[derive(Parser)]
#[command(name = "rustdesk-cli")]
#[command(about = "Command-line RustDesk client for AI agents")]
#[command(version = crate::version::VERSION)]
struct Cli {
    /// Emit machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,

    /// Auto-approve all permission prompts. Dangerous: intended for automation only.
    #[arg(short = 'y', long = "dangerously-skip-permissions", global = true, default_value_t = false)]
    dangerously_skip_permissions: bool,

    /// Enable sandbox restrictions from rustdesk-cli.toml.
    #[arg(long, global = true, default_value_t = false)]
    sandbox: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Connect to a remote RustDesk peer
    Connect {
        /// Peer ID to connect to
        id: String,
        /// Open a direct interactive terminal instead of spawning the daemon session
        #[arg(long, default_value_t = false)]
        terminal: bool,
        /// Password for the peer. Can also be set via RUSTDESK_PASSWORD env var
        #[arg(long, env = "RUSTDESK_PASSWORD")]
        password: Option<String>,
        /// Read password from stdin (one line). Mutually exclusive with --password
        #[arg(long)]
        password_stdin: bool,
        /// Override combined rendezvous/relay server address
        #[arg(long)]
        server: Option<String>,
        /// Override RustDesk ID/rendezvous server address
        #[arg(long = "id-server")]
        id_server: Option<String>,
        /// Override RustDesk relay server address
        #[arg(long = "relay-server")]
        relay_server: Option<String>,
        /// Override RustDesk server public key
        #[arg(long)]
        key: Option<String>,
        /// Connection timeout in seconds
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Disconnect from current peer
    Disconnect,
    /// Open an interactive remote terminal
    Shell,
    /// Execute a command on the remote machine
    Exec {
        /// Command to execute remotely
        #[arg(long)]
        command: String,
        /// Maximum time to wait for command completion in seconds
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        /// Peer ID for a direct one-shot exec without the daemon
        #[arg(long)]
        peer: Option<String>,
        /// Password for a direct one-shot exec
        #[arg(long, env = "RUSTDESK_PASSWORD")]
        password: Option<String>,
        /// Override combined rendezvous/relay server address
        #[arg(long)]
        server: Option<String>,
        /// Override RustDesk ID/rendezvous server address
        #[arg(long = "id-server", alias = "hbbs")]
        id_server: Option<String>,
        /// Override RustDesk relay server address
        #[arg(long = "relay-server", alias = "hbbr", alias = "relay")]
        relay_server: Option<String>,
        /// Override RustDesk server public key
        #[arg(long)]
        key: Option<String>,
    },
    /// Push a local file to the remote machine
    Push {
        /// Local file path to upload
        local_path: String,
        /// Destination path on the remote machine
        remote_path: String,
        /// Peer ID for a direct one-shot push without the daemon
        #[arg(long)]
        peer: Option<String>,
        /// Password for a direct one-shot push
        #[arg(long, env = "RUSTDESK_PASSWORD")]
        password: Option<String>,
        /// Override combined rendezvous/relay server address
        #[arg(long)]
        server: Option<String>,
        /// Override RustDesk ID/rendezvous server address
        #[arg(long = "id-server", alias = "hbbs")]
        id_server: Option<String>,
        /// Override RustDesk relay server address
        #[arg(long = "relay-server", alias = "hbbr", alias = "relay")]
        relay_server: Option<String>,
        /// Override RustDesk server public key
        #[arg(long)]
        key: Option<String>,
        /// Direct connection timeout in seconds
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Get or set remote clipboard text
    Clipboard {
        #[command(subcommand)]
        command: ClipboardCommands,
    },
    /// Show connection status
    Status,
    /// List available displays on the remote peer
    Displays,
    /// Capture a screenshot from the remote display
    Capture {
        /// Output file path
        file: Option<String>,
        /// Display index to capture
        #[arg(long)]
        display: Option<i32>,
        /// Image format
        #[arg(long, value_enum)]
        format: Option<CaptureFormat>,
        /// JPEG quality (1-100)
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
        /// Capture request timeout in seconds
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        /// Capture region as x,y,w,h
        #[arg(long)]
        region: Option<Region>,
    },
    /// Type text on the remote machine
    Type {
        /// Text to type
        text: String,
    },
    /// Send a key press to the remote machine
    Key {
        /// Key name (e.g. enter, tab, a)
        key: String,
        /// Modifier keys as comma-separated values: ctrl,shift,alt
        #[arg(long, value_enum, value_delimiter = ',')]
        modifiers: Vec<Modifier>,
    },
    /// Click at coordinates on the remote display
    Click {
        /// Mouse button (left, right, middle)
        #[arg(long, value_enum, default_value_t = MouseButton::Left)]
        button: MouseButton,
        /// Double-click
        #[arg(long, default_value_t = false)]
        double: bool,
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
    },
    /// Scroll the mouse wheel at coordinates
    #[command(allow_hyphen_values = true)]
    Scroll {
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
        /// Scroll delta (positive=up, negative=down)
        delta: i32,
    },
    /// Move the mouse cursor on the remote display
    Move {
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
    },
    /// Drag from one position to another
    Drag {
        /// Start X coordinate
        x1: i32,
        /// Start Y coordinate
        y1: i32,
        /// End X coordinate
        x2: i32,
        /// End Y coordinate
        y2: i32,
    },
    /// Execute multiple sub-steps in sequence
    Do {
        /// Batch steps, parsed as repeated rustdesk-cli verbs
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        steps: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ClipboardCommands {
    /// Get remote clipboard text
    Get,
    /// Set remote clipboard text
    Set {
        /// Clipboard text to send
        #[arg(long)]
        text: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CaptureFormat {
    Png,
    Jpg,
}

impl CaptureFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

impl Modifier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
            Self::Meta => "meta",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Region {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Region {
    fn to_json(self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "w": self.w,
            "h": self.h
        })
    }

    fn as_text(self) -> String {
        format!("{},{},{},{}", self.x, self.y, self.w, self.h)
    }

    fn to_capture_region(self) -> crate::session::CaptureRegion {
        crate::session::CaptureRegion {
            x: self.x as u32,
            y: self.y as u32,
            w: self.w as u32,
            h: self.h as u32,
        }
    }
}

impl FromStr for Region {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = value.split(',').collect();
        if parts.len() != 4 {
            return Err("region must be in x,y,w,h format".to_string());
        }

        let x = parts[0]
            .parse::<i32>()
            .map_err(|_| "invalid region x coordinate".to_string())?;
        let y = parts[1]
            .parse::<i32>()
            .map_err(|_| "invalid region y coordinate".to_string())?;
        let w = parts[2]
            .parse::<i32>()
            .map_err(|_| "invalid region width".to_string())?;
        let h = parts[3]
            .parse::<i32>()
            .map_err(|_| "invalid region height".to_string())?;

        if w <= 0 || h <= 0 {
            return Err("region width and height must be positive".to_string());
        }

        Ok(Self { x, y, w, h })
    }
}

struct Response {
    text: String,
    json: Value,
    exit_code: i32,
}

struct BatchResponse {
    lines: Vec<String>,
    json: Value,
    exit_code: i32,
}

#[derive(Debug)]
struct BatchStep {
    command: String,
    args: Vec<String>,
}

fn main() {
    // Intercept --daemon mode before clap parsing
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--daemon") {
        run_daemon_mode(&args);
        return;
    }
    process::exit(run());
}

fn run_daemon_mode(args: &[String]) {
    let peer_id = daemon_arg_value(args, "--peer-id")
        .expect("--daemon requires --peer-id");
    let password = daemon_arg_value(args, "--password");
    let server = daemon_arg_value(args, "--server");
    let id_server = daemon_arg_value(args, "--id-server");
    let relay_server = daemon_arg_value(args, "--relay-server");
    let key = daemon_arg_value(args, "--key");
    let connect_timeout =
        daemon_arg_value(args, "--connect-timeout").and_then(|s| s.parse::<u64>().ok());
    let timeout = daemon_arg_value(args, "--timeout").and_then(|s| s.parse::<u64>().ok());
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    if let Err(e) = rt.block_on(daemon::run_daemon(
        peer_id,
        password,
        server,
        id_server,
        relay_server,
        key,
        connect_timeout,
        timeout,
    )) {
        eprintln!("daemon error: {e}");
        process::exit(1);
    }
}

fn daemon_arg_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}

fn build_direct_connection_config(
    peer_id: &str,
    password: Option<&str>,
    server: Option<&str>,
    id_server: Option<&str>,
    relay_server: Option<&str>,
    key: Option<&str>,
) -> connection::ConnectionConfig {
    let id_srv = match id_server {
        Some(s) => s.to_string(),
        None => match server {
            Some(s) => {
                let host = s.split(':').next().unwrap_or(s);
                format!("{host}:21116")
            }
            None => match relay_server {
                Some(s) => infer_id_server_from_relay(s),
                None => "localhost:21116".to_string(),
            },
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

    connection::ConnectionConfig {
        id_server: id_srv,
        relay_server: relay_srv,
        server_key: key.unwrap_or("").to_string(),
        peer_id: peer_id.to_string(),
        password: password.unwrap_or("").to_string(),
        warmup_secs: 2,
    }
}

fn infer_id_server_from_relay(relay_server: &str) -> String {
    let host = relay_server.split(':').next().unwrap_or(relay_server);
    let port = relay_server
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .map(|port| port.saturating_sub(1))
        .filter(|port| *port > 0)
        .unwrap_or(21116);
    format!("{host}:{port}")
}

fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let exit_code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => EXIT_SUCCESS,
                _ => EXIT_INPUT,
            };
            let _ = err.print();
            return exit_code;
        }
    };

    let permissions = match PermissionManager::from_flags(
        cli.dangerously_skip_permissions,
        cli.sandbox,
    ) {
        Ok(permissions) => permissions,
        Err(e) => {
            return emit_response(
                cli.json,
                error_response(
                    "permissions",
                    "permission_error",
                    &e.to_string(),
                    EXIT_PERMISSION,
                ),
            );
        }
    };

    match cli.command {
        Commands::Connect {
            id,
            terminal: terminal_mode,
            password,
            password_stdin,
            server,
            id_server,
            relay_server,
            key,
            timeout,
        } => {
            if password.is_some() && password_stdin {
                eprintln!("error: --password and --password-stdin are mutually exclusive");
                return EXIT_INPUT;
            }
            let password = if password_stdin {
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).is_err() || line.is_empty() {
                    eprintln!("error: failed to read password from stdin");
                    return EXIT_INPUT;
                }
                Some(line.trim_end_matches('\n').to_string())
            } else {
                password
            };

            if let Err(e) = permissions.ensure_connect_allowed(&id) {
                return emit_response(
                    cli.json,
                    error_response(
                        "connect",
                        "permission_error",
                        &e.to_string(),
                        EXIT_PERMISSION,
                    ),
                );
            }

            if terminal_mode {
                if cli.json {
                    return emit_response(
                        cli.json,
                        error_response(
                            "connect",
                            "input_error",
                            "--json is not supported with interactive --terminal mode",
                            EXIT_INPUT,
                        ),
                    );
                }

                let config = build_direct_connection_config(
                    &id,
                    password.as_deref(),
                    server.as_deref(),
                    id_server.as_deref(),
                    relay_server.as_deref(),
                    key.as_deref(),
                );
                let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                return match rt.block_on(crate::terminal::run_terminal_session(&config)) {
                    Ok(()) => EXIT_SUCCESS,
                    Err(e) => emit_response(
                        cli.json,
                        error_response(
                            "connect",
                            "connection_error",
                            &e.to_string(),
                            EXIT_CONNECTION,
                        ),
                    ),
                };
            }

            match daemon::spawn_daemon(
                &id,
                password.as_deref(),
                server.as_deref(),
                id_server.as_deref(),
                relay_server.as_deref(),
                key.as_deref(),
                Some(timeout),
            ) {
                Ok(()) => emit_response(cli.json, connect_response(&id, server.as_deref())),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "connect",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Disconnect => {
            let was_connected = daemon::is_daemon_running();
            if !was_connected {
                return emit_response(
                    cli.json,
                    error_response(
                        "disconnect",
                        "session_error",
                        "No active session",
                        EXIT_SESSION,
                    ),
                );
            }
            let _ = send_to_daemon(&SessionCommand::Disconnect);
            emit_response(cli.json, disconnect_response(true))
        }
        Commands::Shell => {
            if let Err(e) = permissions.ensure_shell_allowed() {
                return emit_response(
                    cli.json,
                    error_response(
                        "shell",
                        "permission_error",
                        &e.to_string(),
                        EXIT_PERMISSION,
                    ),
                );
            }

            match send_to_daemon(&SessionCommand::Shell) {
                Ok(resp) if resp.success => emit_response(cli.json, shell_response()),
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "shell",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("shell failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "shell",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Exec {
            command,
            timeout,
            peer,
            password,
            server,
            id_server,
            relay_server,
            key,
        } => {
            if let Err(e) = permissions.ensure_exec_allowed(&command) {
                return emit_response(
                    cli.json,
                    error_response(
                        "exec",
                        "permission_error",
                        &e.to_string(),
                        EXIT_PERMISSION,
                    ),
                );
            }

            if let Some(peer_id) = peer {
                if let Err(e) = permissions.ensure_connect_allowed(&peer_id) {
                    return emit_response(
                        cli.json,
                        error_response(
                            "exec",
                            "permission_error",
                            &e.to_string(),
                            EXIT_PERMISSION,
                        ),
                    );
                }

                return match direct_exec(
                    &peer_id,
                    password.as_deref(),
                    server.as_deref(),
                    id_server.as_deref(),
                    relay_server.as_deref(),
                    key.as_deref(),
                    timeout,
                    &command,
                    cli.json,
                ) {
                    Ok((stdout, exit_code, timed_out)) => emit_response(
                        cli.json,
                        Response {
                            text: format!("exec exit_code={exit_code} stdout={stdout}"),
                            json: json!({
                                "ok": true,
                                "command": "exec",
                                "data": {
                                    "command": command,
                                    "stdout": stdout,
                                    "stderr": "",
                                    "exit_code": exit_code,
                                    "timed_out": timed_out,
                                }
                            }),
                            exit_code: EXIT_SUCCESS,
                        },
                    ),
                    Err(e) => emit_response(
                        cli.json,
                        error_response(
                            "exec",
                            "connection_error",
                            &e.to_string(),
                            EXIT_CONNECTION,
                        ),
                    ),
                };
            }

            match send_to_daemon(&SessionCommand::Exec {
                command: command.clone(),
                timeout: Some(timeout),
            }) {
                Ok(resp) if resp.success => {
                    let data = resp.data.unwrap_or_else(|| json!({}));
                    let stdout = data
                        .get("stdout")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let stderr = data
                        .get("stderr")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let exit_code = data
                        .get("exit_code")
                        .and_then(Value::as_i64)
                        .unwrap_or(0) as i32;
                    emit_response(cli.json, exec_response(&command, stdout, stderr, exit_code))
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "exec",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("exec failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "exec",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Push {
            local_path,
            remote_path,
            peer,
            password,
            server,
            id_server,
            relay_server,
            key,
            timeout,
        } => {
            if let Err(e) = permissions.ensure_push_allowed(&local_path, &remote_path) {
                return emit_response(
                    cli.json,
                    error_response(
                        "push",
                        "permission_error",
                        &e.to_string(),
                        EXIT_PERMISSION,
                    ),
                );
            }

            if let Some(peer_id) = peer {
                if let Err(e) = permissions.ensure_connect_allowed(&peer_id) {
                    return emit_response(
                        cli.json,
                        error_response(
                            "push",
                            "permission_error",
                            &e.to_string(),
                            EXIT_PERMISSION,
                        ),
                    );
                }

                match direct_push(
                    &peer_id,
                    password.as_deref(),
                    server.as_deref(),
                    id_server.as_deref(),
                    relay_server.as_deref(),
                    key.as_deref(),
                    timeout,
                    &local_path,
                    &remote_path,
                    cli.json,
                ) {
                    Ok((sent_bytes, total_bytes, resumed_bytes)) => {
                        return emit_response(
                            cli.json,
                            push_response(
                                &local_path,
                                &remote_path,
                                sent_bytes,
                                total_bytes,
                                resumed_bytes,
                            ),
                        );
                    }
                    Err(e) => {
                        return emit_response(
                            cli.json,
                            error_response(
                                "push",
                                "connection_error",
                                &e.to_string(),
                                EXIT_CONNECTION,
                            ),
                        );
                    }
                }
            }

            match send_to_daemon_streaming(
                &SessionCommand::Push {
                    local_path: local_path.clone(),
                    remote_path: remote_path.clone(),
                },
                |resp| {
                    if cli.json {
                        return;
                    }
                    let Some(data) = resp.data.as_ref() else {
                        return;
                    };
                    if data.get("kind").and_then(Value::as_str) != Some("progress") {
                        return;
                    }
                    let sent = data.get("sent_bytes").and_then(Value::as_u64).unwrap_or(0);
                    let total = data.get("total_bytes").and_then(Value::as_u64).unwrap_or(0);
                    let resumed = data
                        .get("resumed_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let pct = if total == 0 {
                        100.0
                    } else {
                        (sent as f64 / total as f64) * 100.0
                    };
                    eprint!(
                        "\rpush sent={sent}/{total} bytes ({pct:.1}%) resumed={resumed}"
                    );
                    let _ = std::io::stderr().flush();
                },
            ) {
                Ok(resp) if resp.success => {
                    if !cli.json {
                        eprintln!();
                    }
                    let data = resp.data.unwrap_or_else(|| json!({}));
                    let total_bytes = data
                        .get("total_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let sent_bytes = data
                        .get("sent_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let resumed_bytes = data
                        .get("resumed_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    emit_response(
                        cli.json,
                        push_response(&local_path, &remote_path, sent_bytes, total_bytes, resumed_bytes),
                    )
                }
                Ok(resp) => {
                    if !cli.json {
                        eprintln!();
                    }
                    emit_response(
                        cli.json,
                        error_response(
                            "push",
                            "connection_error",
                            resp.message.as_deref().unwrap_or("push failed"),
                            EXIT_CONNECTION,
                        ),
                    )
                }
                Err(e) => {
                    if !cli.json {
                        eprintln!();
                    }
                    emit_response(
                        cli.json,
                        error_response(
                            "push",
                            "connection_error",
                            &e.to_string(),
                            EXIT_CONNECTION,
                        ),
                    )
                }
            }
        }
        Commands::Clipboard { command } => match command {
            ClipboardCommands::Get => match send_to_daemon(&SessionCommand::ClipboardGet) {
                Ok(resp) if resp.success => {
                    let data = resp.data.unwrap_or_else(|| json!({}));
                    let text = data
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("stub clipboard text");
                    emit_response(cli.json, clipboard_get_response(text))
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "clipboard",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("clipboard get failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "clipboard",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            },
            ClipboardCommands::Set { text } => {
                match send_to_daemon(&SessionCommand::ClipboardSet {
                    text: text.clone(),
                }) {
                    Ok(resp) if resp.success => emit_response(cli.json, clipboard_set_response(&text)),
                    Ok(resp) => emit_response(
                        cli.json,
                        error_response(
                            "clipboard",
                            "connection_error",
                            resp.message.as_deref().unwrap_or("clipboard set failed"),
                            EXIT_CONNECTION,
                        ),
                    ),
                    Err(e) => emit_response(
                        cli.json,
                        error_response(
                            "clipboard",
                            "connection_error",
                            &e.to_string(),
                            EXIT_CONNECTION,
                        ),
                    ),
                }
            }
        },
        Commands::Status => {
            if daemon::is_daemon_running() {
                match send_to_daemon(&SessionCommand::Status) {
                    Ok(resp) if resp.success => {
                        let data = resp.data.unwrap_or(json!({}));
                        let peer_id = data
                            .get("peer_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        emit_response(cli.json, status_connected_response(peer_id))
                    }
                    _ => emit_response(cli.json, status_response()),
                }
            } else {
                emit_response(cli.json, status_response())
            }
        }
        Commands::Displays => match send_to_daemon(&SessionCommand::Displays) {
            Ok(resp) if resp.success => {
                let data = resp.data.unwrap_or(json!({}));
                let displays = data
                    .get("displays")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let text_lines: Vec<String> = displays
                    .iter()
                    .map(|d| {
                        format!(
                            "display {} {}x{} at ({},{}) name={}",
                            d["idx"],
                            d["width"],
                            d["height"],
                            d["x"],
                            d["y"],
                            d["name"].as_str().unwrap_or("")
                        )
                    })
                    .collect();
                emit_response(
                    cli.json,
                    Response {
                        text: if text_lines.is_empty() {
                            "no displays".to_string()
                        } else {
                            text_lines.join("\n")
                        },
                        json: json!({
                            "ok": true,
                            "command": "displays",
                            "displays": displays
                        }),
                        exit_code: EXIT_SUCCESS,
                    },
                )
            }
            Ok(resp) => emit_response(
                cli.json,
                error_response(
                    "displays",
                    "connection_error",
                    resp.message.as_deref().unwrap_or("displays failed"),
                    EXIT_CONNECTION,
                ),
            ),
            Err(e) => emit_response(
                cli.json,
                error_response(
                    "displays",
                    "connection_error",
                    &e.to_string(),
                    EXIT_CONNECTION,
                ),
            ),
        },
        Commands::Capture {
            file,
            display,
            format,
            quality,
            timeout,
            region,
        } => {
            if !daemon::is_daemon_running() {
                if let Some(file) = file.as_deref() {
                    return emit_response(
                        cli.json,
                        capture_response(
                            file,
                            format.unwrap_or_else(|| infer_format(file)),
                            region,
                            display,
                            timeout,
                        ),
                    );
                }
                return capture::write_capture_output(fake_capture_payload(CaptureFormat::Png), None)
                    .map(|_| EXIT_SUCCESS)
                    .unwrap_or_else(|e| {
                        emit_response(
                            cli.json,
                            error_response(
                                "capture",
                                "connection_error",
                                &e.to_string(),
                                EXIT_CONNECTION,
                            ),
                        )
                    });
            }

            let output = file.clone().unwrap_or_default();
            let response_format = format
                .or_else(|| file.as_deref().map(infer_format))
                .unwrap_or(CaptureFormat::Png);

            match send_to_daemon(&SessionCommand::Capture {
                output,
                format: Some(response_format.as_str().to_string()),
                quality: Some(quality),
                region: region.map(Region::to_capture_region),
                display,
                timeout_secs: Some(timeout),
            }) {
                Ok(resp) if resp.success => {
                    let Some(data) = resp.data else {
                        return emit_response(
                            cli.json,
                            error_response(
                                "capture",
                                "connection_error",
                                "capture response missing image bytes",
                                EXIT_CONNECTION,
                            ),
                        );
                    };

                    let encoded = data
                        .get("bytes_b64")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let bytes = match capture::base64_decode(encoded) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            return emit_response(
                                cli.json,
                                error_response(
                                    "capture",
                                    "connection_error",
                                    &format!("invalid capture payload: {e}"),
                                    EXIT_CONNECTION,
                                ),
                            )
                        }
                    };

                    if let Err(e) = capture::write_capture_output(&bytes, file.as_deref()) {
                        return emit_response(
                            cli.json,
                            error_response(
                                "capture",
                                "connection_error",
                                &e.to_string(),
                                EXIT_CONNECTION,
                            ),
                        );
                    }

                    if file.is_none() && !cli.json {
                        EXIT_SUCCESS
                    } else {
                        emit_response(
                            cli.json,
                            capture_result_response(
                                file.as_deref(),
                                response_format,
                                region,
                                display,
                                timeout,
                                bytes.len(),
                            ),
                        )
                    }
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "capture",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("capture failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "capture",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Type { text } => {
            match send_to_daemon(&SessionCommand::Type { text: text.clone() }) {
                Ok(resp) if resp.success => emit_response(cli.json, type_response(&text)),
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "type",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("type failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "type",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Key { key, modifiers } => {
            match send_to_daemon(&SessionCommand::Key { key: key.clone() }) {
                Ok(resp) if resp.success => {
                    emit_response(cli.json, key_response(&key, &modifiers))
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "key",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("key failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "key",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Click { button, double, x, y } => {
            match send_to_daemon(&SessionCommand::Click {
                x,
                y,
                button: button.as_str().to_string(),
                double,
            }) {
                Ok(resp) if resp.success => {
                    emit_response(cli.json, click_response(button, x, y, double))
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "click",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("click failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "click",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Scroll { x, y, delta } => {
            match send_to_daemon(&SessionCommand::Scroll { x, y, delta }) {
                Ok(resp) if resp.success => {
                    emit_response(cli.json, scroll_response(x, y, delta))
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "scroll",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("scroll failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "scroll",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Move { x, y } => match send_to_daemon(&SessionCommand::Move { x, y }) {
            Ok(resp) if resp.success => emit_response(cli.json, move_response(x, y)),
            Ok(resp) => emit_response(
                cli.json,
                error_response(
                    "move",
                    "connection_error",
                    resp.message.as_deref().unwrap_or("move failed"),
                    EXIT_CONNECTION,
                ),
            ),
            Err(e) => emit_response(
                cli.json,
                error_response(
                    "move",
                    "connection_error",
                    &e.to_string(),
                    EXIT_CONNECTION,
                ),
            ),
        },
        Commands::Drag { x1, y1, x2, y2 } => {
            match send_to_daemon(&SessionCommand::Drag {
                x: x1,
                y: y1,
                x2,
                y2,
                button: "left".to_string(),
            }) {
                Ok(resp) if resp.success => {
                    emit_response(cli.json, drag_response(x1, y1, x2, y2))
                }
                Ok(resp) => emit_response(
                    cli.json,
                    error_response(
                        "drag",
                        "connection_error",
                        resp.message.as_deref().unwrap_or("drag failed"),
                        EXIT_CONNECTION,
                    ),
                ),
                Err(e) => emit_response(
                    cli.json,
                    error_response(
                        "drag",
                        "connection_error",
                        &e.to_string(),
                        EXIT_CONNECTION,
                    ),
                ),
            }
        }
        Commands::Do { steps } => {
            if !daemon::is_daemon_running() {
                return emit_batch_response(
                    cli.json,
                    BatchResponse {
                        lines: vec!["session_error: No active session".to_string()],
                        json: json!({
                            "ok": false,
                            "command": "do",
                            "error": {
                                "code": "session_error",
                                "message": "No active session"
                            }
                        }),
                        exit_code: EXIT_SESSION,
                    },
                );
            }

            let response = match parse_batch_steps(&steps) {
                Ok(parsed_steps) => do_response(&parsed_steps),
                Err(message) => batch_error_response(message),
            };
            emit_batch_response(cli.json, response)
        }
    }
}

fn send_to_daemon(cmd: &SessionCommand) -> Result<SessionResponse, anyhow::Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(daemon::send_command(cmd))
}

fn send_to_daemon_streaming(
    cmd: &SessionCommand,
    mut on_response: impl FnMut(&SessionResponse),
) -> Result<SessionResponse, anyhow::Error> {
    let lock = daemon::LockFile::read()?;
    let mut stream = StdUnixStream::connect(&lock.socket)?;
    let mut payload = serde_json::to_vec(cmd)?;
    payload.push(b'\n');
    stream.write_all(&payload)?;
    stream.shutdown(Shutdown::Write)?;

    let mut last_response = None;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let response: SessionResponse = serde_json::from_str(line.trim())?;
        on_response(&response);
        last_response = Some(response);
    }

    last_response.ok_or_else(|| anyhow::anyhow!("daemon closed without sending a response"))
}

fn direct_push(
    peer_id: &str,
    password: Option<&str>,
    server: Option<&str>,
    id_server: Option<&str>,
    relay_server: Option<&str>,
    key: Option<&str>,
    timeout_secs: u64,
    local_path: &str,
    remote_path: &str,
    json_mode: bool,
) -> Result<(u64, u64, u64), anyhow::Error> {
    let config = build_direct_connection_config(
        peer_id,
        password,
        server,
        id_server,
        relay_server,
        key,
    );
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let remote_dir = crate::file_transfer::remote_target_dir(remote_path, Path::new(local_path))?;
        let connection = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            connection::connect_with_mode(
                &config,
                crate::proto::hbb::ConnType::FileTransfer,
                Some(crate::proto::hbb::login_request::Union::FileTransfer(
                    crate::proto::hbb::FileTransfer {
                        dir: remote_dir,
                        show_hidden: false,
                    },
                )),
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("direct push timed out after {timeout_secs}s"))??;

        let mut encrypted = connection.encrypted;
        let mut transfer =
            crate::file_transfer::PushTransfer::begin(&mut encrypted, Path::new(local_path), remote_path)
                .await?;

        if !json_mode {
            render_push_progress(transfer.progress());
        }

        while transfer.send_next_block(&mut encrypted).await? {
            if !json_mode {
                render_push_progress(transfer.progress());
            }
        }
        transfer.wait_for_done(&mut encrypted).await?;
        let result = transfer.result();
        let _ = encrypted.close().await;
        if !json_mode {
            eprintln!();
        }
        Ok((result.sent_bytes, result.total_bytes, result.resumed_bytes))
    })
}

fn direct_exec(
    peer_id: &str,
    password: Option<&str>,
    server: Option<&str>,
    id_server: Option<&str>,
    relay_server: Option<&str>,
    key: Option<&str>,
    timeout_secs: u64,
    command: &str,
    _json_mode: bool,
) -> Result<(String, i32, bool), anyhow::Error> {
    let config = build_direct_connection_config(
        peer_id,
        password,
        server,
        id_server,
        relay_server,
        key,
    );
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let connection = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            connection::connect_with_mode(
                &config,
                crate::proto::hbb::ConnType::Terminal,
                Some(crate::proto::hbb::login_request::Union::Terminal(
                    crate::proto::hbb::Terminal {
                        service_id: String::new(),
                    },
                )),
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("direct exec timed out after {timeout_secs}s"))??;

        let mut encrypted = connection.encrypted;
        let terminal_info = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            crate::terminal::open_terminal(&mut encrypted, 24, 80),
        )
        .await
        .map_err(|_| anyhow::anyhow!("terminal open timed out"))??;
        let tid = terminal_info.terminal_id;

        loop {
            match crate::terminal::recv_terminal_data_with_timeout(
                &mut encrypted,
                std::time::Duration::from_millis(500),
            )
            .await
            {
                Ok(crate::terminal::TerminalEvent::Data(_)) => {}
                Ok(crate::terminal::TerminalEvent::Closed { exit_code }) => {
                    return Ok((String::new(), exit_code, false));
                }
                Ok(crate::terminal::TerminalEvent::Error(msg)) => {
                    let _ = crate::terminal::close_terminal(&mut encrypted, tid).await;
                    anyhow::bail!("terminal error during prompt drain: {msg}");
                }
                Err(e) if crate::terminal::is_terminal_response_timeout(&e) => break,
                Err(e) => {
                    let _ = crate::terminal::close_terminal(&mut encrypted, tid).await;
                    anyhow::bail!("recv error during prompt drain: {e:#}");
                }
            }
        }

        let sentinel_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sentinel = format!("__RDCLI_{sentinel_id:032x}__");
        let wrapped = format!("{command}\necho '{sentinel}'$?\n");
        crate::terminal::send_terminal_data(&mut encrypted, tid, wrapped.as_bytes()).await?;

        let completion_timeout = std::time::Duration::from_secs(timeout_secs);
        let deadline = tokio::time::Instant::now() + completion_timeout;
        let mut collected = Vec::new();
        let mut timed_out = false;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                timed_out = true;
                break;
            }

            match crate::terminal::recv_terminal_data_with_timeout(&mut encrypted, remaining).await
            {
                Ok(crate::terminal::TerminalEvent::Data(data)) => {
                    collected.extend_from_slice(&data);
                    if find_sentinel_output(&String::from_utf8_lossy(&collected), &sentinel)
                        .is_some()
                    {
                        break;
                    }
                }
                Ok(crate::terminal::TerminalEvent::Closed { exit_code }) => {
                    let stdout = String::from_utf8_lossy(&collected).trim().to_string();
                    let _ = crate::terminal::close_terminal(&mut encrypted, tid).await;
                    return Ok((stdout, exit_code, false));
                }
                Ok(crate::terminal::TerminalEvent::Error(msg)) => {
                    let _ = crate::terminal::close_terminal(&mut encrypted, tid).await;
                    anyhow::bail!("terminal error during exec: {msg}");
                }
                Err(e) if crate::terminal::is_terminal_response_timeout(&e) => {
                    timed_out = true;
                    break;
                }
                Err(e) => {
                    let _ = crate::terminal::close_terminal(&mut encrypted, tid).await;
                    anyhow::bail!("recv error during exec: {e:#}");
                }
            }
        }

        let _ = crate::terminal::close_terminal(&mut encrypted, tid).await;
        let _ = encrypted.close().await;
        let raw = String::from_utf8_lossy(&collected);
        let (stdout, exit_code) = parse_direct_exec_output(&raw, &sentinel);
        Ok((stdout, exit_code, timed_out))
    })
}

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

fn parse_direct_exec_output(raw: &str, sentinel: &str) -> (String, i32) {
    if let Some(pos) = find_sentinel_output(raw, sentinel) {
        let before = &raw[..pos];
        let after = &raw[pos + sentinel.len()..];
        let exit_digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let exit_code = exit_digits.parse::<i32>().unwrap_or(0);
        let stdout = before
            .lines()
            .filter(|line| !line.contains(&format!("echo '{sentinel}'$?")))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        return (stdout, exit_code);
    }

    (raw.trim().to_string(), -1)
}

fn render_push_progress(progress: crate::file_transfer::PushProgress) {
    let pct = if progress.total_bytes == 0 {
        100.0
    } else {
        (progress.sent_bytes as f64 / progress.total_bytes as f64) * 100.0
    };
    eprint!(
        "\rpush sent={}/{} bytes ({pct:.1}%) resumed={}",
        progress.sent_bytes,
        progress.total_bytes,
        progress.resumed_bytes
    );
    let _ = std::io::stderr().flush();
}

fn error_response(command: &str, code: &str, message: &str, exit_code: i32) -> Response {
    Response {
        text: format!("{code}: {message}"),
        json: json!({
            "ok": false,
            "command": command,
            "error": {
                "code": code,
                "message": message
            }
        }),
        exit_code,
    }
}

fn emit_response(json_mode: bool, response: Response) -> i32 {
    let is_error = response.exit_code != EXIT_SUCCESS;

    if json_mode {
        let rendered = serde_json::to_string(&response.json).expect("serialize response");
        println!("{rendered}");
    } else if !response.text.is_empty() {
        if is_error {
            eprintln!("{}", response.text);
        } else {
            println!("{}", response.text);
        }
    }

    response.exit_code
}

fn emit_batch_response(json_mode: bool, response: BatchResponse) -> i32 {
    let is_error = response.exit_code != EXIT_SUCCESS;

    if json_mode {
        let rendered = serde_json::to_string(&response.json).expect("serialize batch response");
        println!("{rendered}");
    } else {
        for line in response.lines {
            if is_error {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
    }

    response.exit_code
}

fn connect_response(id: &str, server: Option<&str>) -> Response {
    let mut text = format!("connected id={id}");
    if let Some(server) = server {
        text.push_str(&format!(" server={server}"));
    }
    text.push_str(&format!(" width={DEFAULT_WIDTH} height={DEFAULT_HEIGHT}"));

    Response {
        text,
        json: json!({
            "ok": true,
            "command": "connect",
            "id": id,
            "server": server,
            "connected": true,
            "width": DEFAULT_WIDTH,
            "height": DEFAULT_HEIGHT
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn disconnect_response(was_connected: bool) -> Response {
    Response {
        text: "disconnected".to_string(),
        json: json!({
            "ok": true,
            "command": "disconnect",
            "was_connected": was_connected
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn shell_response() -> Response {
    Response {
        text: "shell mode=interactive".to_string(),
        json: json!({
            "ok": true,
            "command": "shell",
            "mode": "interactive"
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn exec_response(command: &str, stdout: &str, stderr: &str, exit_code: i32) -> Response {
    Response {
        text: format!("exec exit_code={exit_code} stdout={stdout}"),
        json: json!({
            "ok": true,
            "command": "exec",
            "requested": command,
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn push_response(
    local_path: &str,
    remote_path: &str,
    sent_bytes: u64,
    total_bytes: u64,
    resumed_bytes: u64,
) -> Response {
    Response {
        text: format!(
            "push sent_bytes={sent_bytes} total_bytes={total_bytes} resumed_bytes={resumed_bytes} remote={remote_path}"
        ),
        json: json!({
            "ok": true,
            "command": "push",
            "local_path": local_path,
            "remote_path": remote_path,
            "sent_bytes": sent_bytes,
            "total_bytes": total_bytes,
            "resumed_bytes": resumed_bytes
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn clipboard_get_response(text: &str) -> Response {
    Response {
        text: format!("clipboard text={text}"),
        json: json!({
            "ok": true,
            "command": "clipboard",
            "action": "get",
            "text": text
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn clipboard_set_response(text: &str) -> Response {
    let chars = text.chars().count();

    Response {
        text: format!("clipboard_set chars={chars}"),
        json: json!({
            "ok": true,
            "command": "clipboard",
            "action": "set",
            "chars": chars,
            "redacted": true
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn status_response() -> Response {
    Response {
        text: "disconnected".to_string(),
        json: json!({
            "ok": true,
            "command": "status",
            "connected": false
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn status_connected_response(peer_id: &str) -> Response {
    Response {
        text: format!(
            "connected id={peer_id} width={DEFAULT_WIDTH} height={DEFAULT_HEIGHT}"
        ),
        json: json!({
            "ok": true,
            "command": "status",
            "connected": true,
            "id": peer_id,
            "width": DEFAULT_WIDTH,
            "height": DEFAULT_HEIGHT
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn displays_response(displays: &[Value]) -> Response {
    let text_lines: Vec<String> = displays
        .iter()
        .map(|d| {
            format!(
                "display {} {}x{} at ({},{}) name={} online={} cursor_embedded={}",
                d["idx"], d["width"], d["height"], d["x"], d["y"],
                d["name"].as_str().unwrap_or(""),
                d["online"].as_bool().unwrap_or(false),
                d["cursor_embedded"].as_bool().unwrap_or(false),
            )
        })
        .collect();
    Response {
        text: if text_lines.is_empty() {
            "no displays".to_string()
        } else {
            text_lines.join("\n")
        },
        json: json!({
            "ok": true,
            "command": "displays",
            "displays": displays
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn capture_response(
    file: &str,
    format: CaptureFormat,
    region: Option<Region>,
    display: Option<i32>,
    timeout: u64,
) -> Response {
    let (width, height) = match region {
        Some(region) => (region.w, region.h),
        None => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    };
    let bytes = fake_capture_bytes(format, width, height);

    let mut text = format!(
        "captured file={file} format={} width={width} height={height} bytes={bytes}",
        format.as_str()
    );
    if let Some(region) = region {
        text.push_str(&format!(" region={}", region.as_text()));
    }
    if let Some(display) = display {
        text.push_str(&format!(" display={display}"));
    }
    text.push_str(&format!(" timeout={timeout}"));

    let mut json = if let Some(region) = region {
        json!({
            "ok": true,
            "command": "capture",
            "file": file,
            "format": format.as_str(),
            "width": width,
            "height": height,
            "bytes": bytes,
            "region": region.to_json()
        })
    } else {
        json!({
            "ok": true,
            "command": "capture",
            "file": file,
            "format": format.as_str(),
            "width": width,
            "height": height,
            "bytes": bytes
        })
    };

    if let Some(display) = display {
        if let Some(object) = json.as_object_mut() {
            object.insert("display".to_string(), json!(display));
        }
    }
    if let Some(object) = json.as_object_mut() {
        object.insert("timeout".to_string(), json!(timeout));
    }

    Response {
        text,
        json,
        exit_code: EXIT_SUCCESS,
    }
}

fn capture_result_response(
    file: Option<&str>,
    format: CaptureFormat,
    region: Option<Region>,
    display: Option<i32>,
    timeout: u64,
    bytes: usize,
) -> Response {
    let mut json = json!({
        "ok": true,
        "command": "capture",
        "format": format.as_str(),
        "bytes": bytes,
    });

    let text = if let Some(file) = file {
        if let Some(object) = json.as_object_mut() {
            object.insert("file".to_string(), json!(file));
        }
        format!("captured file={file} format={} bytes={bytes}", format.as_str())
    } else {
        format!("captured stdout format={} bytes={bytes}", format.as_str())
    };

    if let Some(region) = region {
        if let Some(object) = json.as_object_mut() {
            object.insert("region".to_string(), region.to_json());
        }
    }
    if let Some(display) = display {
        if let Some(object) = json.as_object_mut() {
            object.insert("display".to_string(), json!(display));
        }
    }
    if let Some(object) = json.as_object_mut() {
        object.insert("timeout".to_string(), json!(timeout));
    }

    Response {
        text,
        json,
        exit_code: EXIT_SUCCESS,
    }
}

fn type_response(text: &str) -> Response {
    let chars = text.chars().count();

    Response {
        text: format!("typed chars={chars}"),
        json: json!({
            "ok": true,
            "command": "type",
            "chars": chars,
            "redacted": true
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn key_response(key: &str, modifiers: &[Modifier]) -> Response {
    let modifier_names: Vec<_> = modifiers.iter().map(|modifier| modifier.as_str()).collect();
    let text = if modifier_names.is_empty() {
        format!("key key={key}")
    } else {
        format!("key key={key} modifiers={}", modifier_names.join(","))
    };

    Response {
        text,
        json: json!({
            "ok": true,
            "command": "key",
            "key": key,
            "modifiers": modifier_names
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn click_response(button: MouseButton, x: i32, y: i32, double: bool) -> Response {
    Response {
        text: format!(
            "clicked button={} x={x} y={y} double={double}",
            button.as_str()
        ),
        json: json!({
            "ok": true,
            "command": "click",
            "button": button.as_str(),
            "x": x,
            "y": y,
            "double": double
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn scroll_response(x: i32, y: i32, delta: i32) -> Response {
    Response {
        text: format!("scrolled x={x} y={y} delta={delta}"),
        json: json!({
            "ok": true,
            "command": "scroll",
            "x": x,
            "y": y,
            "delta": delta
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn move_response(x: i32, y: i32) -> Response {
    Response {
        text: format!("moved x={x} y={y}"),
        json: json!({
            "ok": true,
            "command": "move",
            "x": x,
            "y": y
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn drag_response(x1: i32, y1: i32, x2: i32, y2: i32) -> Response {
    Response {
        text: format!("dragged x1={x1} y1={y1} x2={x2} y2={y2}"),
        json: json!({
            "ok": true,
            "command": "drag",
            "x1": x1,
            "y1": y1,
            "x2": x2,
            "y2": y2,
            "button": "left"
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn do_response(steps: &[BatchStep]) -> BatchResponse {
    let mut lines = Vec::with_capacity(steps.len() + 1);
    let mut json_steps = Vec::with_capacity(steps.len());

    for (index, step) in steps.iter().enumerate() {
        let step_result = step_to_response(step);
        lines.push(format!("{} {}", index + 1, step_result.text));

        let mut step_json = step_result.json;
        if let Some(object) = step_json.as_object_mut() {
            object.insert("index".to_string(), json!(index + 1));
        }
        json_steps.push(step_json);
    }

    lines.push(format!("ok steps={}", steps.len()));

    BatchResponse {
        lines,
        json: json!({
            "ok": true,
            "command": "do",
            "steps": json_steps
        }),
        exit_code: EXIT_SUCCESS,
    }
}

fn batch_error_response(message: String) -> BatchResponse {
    BatchResponse {
        lines: vec![format!("connection error: {message}")],
        json: json!({
            "ok": false,
            "command": "do",
            "error": {
                "code": "connection_error",
                "message": message
            }
        }),
        exit_code: EXIT_CONNECTION,
    }
}

fn step_to_response(step: &BatchStep) -> Response {
    match step.command.as_str() {
        "connect" => {
            let id = first_non_flag_arg(&step.args).unwrap_or("unknown");
            let server = flag_value(&step.args, "--server");
            connect_response(id, server)
        }
        "disconnect" => disconnect_response(false),
        "shell" => shell_response(),
        "exec" => {
            let command = flag_value(&step.args, "--command").unwrap_or("");
            exec_response(command, "stub exec output", "", 0)
        }
        "clipboard" => {
            let action = first_non_flag_arg(&step.args).unwrap_or("get");
            match action {
                "get" => clipboard_get_response("stub clipboard text"),
                "set" => clipboard_set_response(flag_value(&step.args, "--text").unwrap_or("")),
                _ => Response {
                    text: "unknown clipboard action".to_string(),
                    json: json!({
                        "ok": false,
                        "command": "clipboard",
                        "error": {
                            "code": "connection_error",
                            "message": "unknown clipboard action"
                        }
                    }),
                    exit_code: EXIT_CONNECTION,
                },
            }
        }
        "status" => status_response(),
        "displays" => displays_response(&[]),
        "capture" => {
            let file = first_non_flag_arg(&step.args).unwrap_or("screenshot.png");
            let format = flag_value(&step.args, "--format")
                .and_then(parse_capture_format)
                .unwrap_or_else(|| infer_format(file));
            let region = flag_value(&step.args, "--region").and_then(|raw| raw.parse::<Region>().ok());
            let display = flag_value(&step.args, "--display").and_then(|raw| raw.parse::<i32>().ok());
            let timeout = flag_value(&step.args, "--timeout")
                .and_then(|raw| raw.parse::<u64>().ok())
                .unwrap_or(10);
            capture_response(file, format, region, display, timeout)
        }
        "type" => type_response(step.args.first().map(String::as_str).unwrap_or("")),
        "key" => {
            let key = first_non_flag_arg(&step.args).unwrap_or("enter");
            let modifiers = flag_value(&step.args, "--modifiers")
                .map(parse_modifier_list)
                .unwrap_or_default();
            key_response(key, &modifiers)
        }
        "click" => {
            let button = flag_value(&step.args, "--button")
                .and_then(parse_mouse_button)
                .unwrap_or(MouseButton::Left);
            let double = flag_present(&step.args, "--double");
            let coords = positional_args(&step.args);
            let x = coords.first().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let y = coords.get(1).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            click_response(button, x, y, double)
        }
        "scroll" => {
            let x = step.args.first().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let y = step.args.get(1).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let delta = step.args.get(2).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            scroll_response(x, y, delta)
        }
        "move" => {
            let x = step.args.first().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let y = step.args.get(1).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            move_response(x, y)
        }
        "drag" => {
            let x1 = step.args.first().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let y1 = step.args.get(1).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let x2 = step.args.get(2).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let y2 = step.args.get(3).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            drag_response(x1, y1, x2, y2)
        }
        _ => Response {
            text: format!("unknown command {}", step.command),
            json: json!({
                "ok": false,
                "command": step.command,
                "error": {
                    "code": "connection_error",
                    "message": "unknown batch command"
                }
            }),
            exit_code: EXIT_CONNECTION,
        },
    }
}

fn infer_format(file: &str) -> CaptureFormat {
    if file.rsplit('.').next().is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg")) {
        CaptureFormat::Jpg
    } else {
        CaptureFormat::Png
    }
}

fn fake_capture_bytes(format: CaptureFormat, width: i32, height: i32) -> u64 {
    let pixels = (width as u64) * (height as u64);
    match format {
        CaptureFormat::Png => pixels / 8 + 8_193,
        CaptureFormat::Jpg => pixels / 12 + 4_821,
    }
}

fn fake_capture_payload(format: CaptureFormat) -> &'static [u8] {
    match format {
        CaptureFormat::Png => &[
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I',
            b'H', b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
            0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, b'I', b'D',
            b'A', b'T', 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00,
            0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N',
            b'D', 0xAE, 0x42, 0x60, 0x82,
        ],
        CaptureFormat::Jpg => &[
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01,
            0x01, 0x00, 0x48, 0x00, 0x48, 0x00, 0x00, 0xFF, 0xD9,
        ],
    }
}

fn parse_batch_steps(tokens: &[String]) -> Result<Vec<BatchStep>, String> {
    let mut index = 0;
    let mut steps = Vec::new();

    while index < tokens.len() {
        let command = tokens[index].clone();
        if !is_step_command(&command) {
            return Err(format!("unknown batch command '{command}'"));
        }

        match command.as_str() {
            "disconnect" | "status" | "displays" | "shell" => {
                steps.push(BatchStep {
                    command,
                    args: Vec::new(),
                });
                index += 1;
            }
            "type" => {
                if index + 1 >= tokens.len() {
                    return Err("type requires one text argument".to_string());
                }
                steps.push(BatchStep {
                    command,
                    args: vec![tokens[index + 1].clone()],
                });
                index += 2;
            }
            "scroll" => {
                if index + 3 >= tokens.len() {
                    return Err("scroll requires x y delta".to_string());
                }
                steps.push(BatchStep {
                    command,
                    args: vec![
                        tokens[index + 1].clone(),
                        tokens[index + 2].clone(),
                        tokens[index + 3].clone(),
                    ],
                });
                index += 4;
            }
            "move" => {
                if index + 2 >= tokens.len() {
                    return Err("move requires x and y".to_string());
                }
                steps.push(BatchStep {
                    command,
                    args: vec![tokens[index + 1].clone(), tokens[index + 2].clone()],
                });
                index += 3;
            }
            "drag" => {
                if index + 4 >= tokens.len() {
                    return Err("drag requires x1 y1 x2 y2".to_string());
                }
                steps.push(BatchStep {
                    command,
                    args: vec![
                        tokens[index + 1].clone(),
                        tokens[index + 2].clone(),
                        tokens[index + 3].clone(),
                        tokens[index + 4].clone(),
                    ],
                });
                index += 5;
            }
            "click" => {
                let (args, next_index) = parse_click_step(tokens, index + 1)?;
                steps.push(BatchStep { command, args });
                index = next_index;
            }
            "clipboard" => {
                let (args, next_index) = parse_clipboard_step(tokens, index + 1)?;
                steps.push(BatchStep { command, args });
                index = next_index;
            }
            "connect" => {
                let (args, next_index) = parse_flagged_step(tokens, index + 1, &["--password", "--server", "--timeout"], 1)?;
                steps.push(BatchStep { command, args });
                index = next_index;
            }
            "exec" => {
                let (args, next_index) = parse_flagged_step(tokens, index + 1, &["--command"], 0)?;
                if flag_value(&args, "--command").is_none() {
                    return Err("exec requires --command <CMD>".to_string());
                }
                steps.push(BatchStep { command, args });
                index = next_index;
            }
            "key" => {
                let (args, next_index) = parse_flagged_step(tokens, index + 1, &["--modifiers"], 1)?;
                steps.push(BatchStep { command, args });
                index = next_index;
            }
            "capture" => {
                let (args, next_index) =
                    parse_flagged_step(tokens, index + 1, &["--display", "--format", "--quality", "--region", "--timeout"], 0)?;
                steps.push(BatchStep { command, args });
                index = next_index;
            }
            _ => return Err(format!("unknown batch command '{command}'")),
        }
    }

    Ok(steps)
}

fn parse_click_step(tokens: &[String], mut index: usize) -> Result<(Vec<String>, usize), String> {
    let mut args = Vec::new();
    let mut positional = 0;

    while index < tokens.len() {
        let token = &tokens[index];
        if positional >= 2 && is_step_command(token) {
            break;
        }

        if token == "--button" {
            if index + 1 >= tokens.len() {
                return Err("click flag --button requires a value".to_string());
            }
            args.push(token.clone());
            args.push(tokens[index + 1].clone());
            index += 2;
            continue;
        }

        if token == "--double" {
            args.push(token.clone());
            index += 1;
            continue;
        }

        if token.starts_with("--") {
            return Err(format!("unsupported click flag '{token}'"));
        }

        args.push(token.clone());
        positional += 1;
        index += 1;

        if positional >= 2 && index < tokens.len() && is_step_command(&tokens[index]) {
            break;
        }
    }

    if positional < 2 {
        return Err("click requires x and y".to_string());
    }

    Ok((args, index))
}

fn parse_clipboard_step(tokens: &[String], mut index: usize) -> Result<(Vec<String>, usize), String> {
    if index >= tokens.len() {
        return Err("clipboard requires an action".to_string());
    }

    let action = tokens[index].clone();
    index += 1;

    match action.as_str() {
        "get" => Ok((vec![action], index)),
        "set" => {
            let (mut args, next_index) = parse_flagged_step(tokens, index, &["--text"], 0)?;
            if flag_value(&args, "--text").is_none() {
                return Err("clipboard set requires --text <TEXT>".to_string());
            }

            let mut full_args = vec![action];
            full_args.append(&mut args);
            Ok((full_args, next_index))
        }
        _ => Err(format!("unknown clipboard action '{action}'")),
    }
}

fn parse_flagged_step(
    tokens: &[String],
    mut index: usize,
    value_flags: &[&str],
    required_positionals: usize,
) -> Result<(Vec<String>, usize), String> {
    let mut args = Vec::new();
    let mut positional = 0;

    while index < tokens.len() {
        let token = &tokens[index];
        if positional >= required_positionals && is_step_command(token) {
            break;
        }

        if value_flags.contains(&token.as_str()) {
            if index + 1 >= tokens.len() {
                return Err(format!("flag '{token}' requires a value"));
            }
            args.push(token.clone());
            args.push(tokens[index + 1].clone());
            index += 2;
            continue;
        }

        if token.starts_with("--") {
            return Err(format!("unsupported flag '{token}'"));
        }

        args.push(token.clone());
        positional += 1;
        index += 1;
    }

    if positional < required_positionals {
        return Err("missing required positional argument".to_string());
    }

    Ok((args, index))
}

fn is_step_command(token: &str) -> bool {
    matches!(
        token,
        "connect"
            | "disconnect"
            | "shell"
            | "exec"
            | "clipboard"
            | "status"
            | "displays"
            | "capture"
            | "type"
            | "key"
            | "click"
            | "scroll"
            | "move"
            | "drag"
    )
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}

fn flag_present(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn first_non_flag_arg(args: &[String]) -> Option<&str> {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        if arg.starts_with("--") {
            skip_next = true;
            continue;
        }

        return Some(arg.as_str());
    }

    None
}

fn positional_args(args: &[String]) -> Vec<&str> {
    let mut positionals = Vec::new();
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        if arg.starts_with("--") {
            skip_next = true;
            continue;
        }

        positionals.push(arg.as_str());
    }

    positionals
}

fn parse_capture_format(value: &str) -> Option<CaptureFormat> {
    match value {
        "png" => Some(CaptureFormat::Png),
        "jpg" | "jpeg" => Some(CaptureFormat::Jpg),
        _ => None,
    }
}

fn parse_mouse_button(value: &str) -> Option<MouseButton> {
    match value {
        "left" => Some(MouseButton::Left),
        "right" => Some(MouseButton::Right),
        "middle" => Some(MouseButton::Middle),
        _ => None,
    }
}

fn parse_modifier_list(value: &str) -> Vec<Modifier> {
    value
        .split(',')
        .filter_map(|item| match item {
            "ctrl" => Some(Modifier::Ctrl),
            "shift" => Some(Modifier::Shift),
            "alt" => Some(Modifier::Alt),
            _ => None,
        })
        .collect()
}
