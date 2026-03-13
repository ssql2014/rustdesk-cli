use std::{process, str::FromStr};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

const EXIT_SUCCESS: i32 = 0;
const EXIT_CONNECTION: i32 = 1;
#[allow(dead_code)]
const EXIT_AUTH: i32 = 2;
#[allow(dead_code)]
const EXIT_TIMEOUT: i32 = 3;

const DEFAULT_WIDTH: i32 = 1920;
const DEFAULT_HEIGHT: i32 = 1080;

#[derive(Parser)]
#[command(name = "rustdesk-cli")]
#[command(about = "Command-line RustDesk client for AI agents")]
struct Cli {
    /// Emit machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Connect to a remote RustDesk peer
    Connect {
        /// Peer ID to connect to
        id: String,
        /// Password for the peer
        #[arg(long)]
        password: Option<String>,
        /// Override rendezvous/relay server address
        #[arg(long)]
        server: Option<String>,
        /// Connection timeout in seconds
        #[arg(long, default_value_t = 15)]
        timeout: u64,
    },
    /// Disconnect from current peer
    Disconnect,
    /// Show connection status
    Status,
    /// Capture a screenshot from the remote display
    Capture {
        /// Output file path
        file: String,
        /// Image format
        #[arg(long, value_enum)]
        format: Option<CaptureFormat>,
        /// JPEG quality (1-100)
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
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
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
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
}

impl Modifier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
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
    process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    match cli.command {
        Commands::Connect {
            id,
            password: _,
            server,
            timeout: _,
        } => emit_response(cli.json, connect_response(&id, server.as_deref())),
        Commands::Disconnect => emit_response(cli.json, disconnect_response()),
        Commands::Status => emit_response(cli.json, status_response()),
        Commands::Capture {
            file,
            format,
            quality: _,
            region,
        } => {
            let format = format.unwrap_or_else(|| infer_format(&file));
            emit_response(cli.json, capture_response(&file, format, region))
        }
        Commands::Type { text } => emit_response(cli.json, type_response(&text)),
        Commands::Key { key, modifiers } => emit_response(cli.json, key_response(&key, &modifiers)),
        Commands::Click { button, x, y } => emit_response(cli.json, click_response(button, x, y)),
        Commands::Move { x, y } => emit_response(cli.json, move_response(x, y)),
        Commands::Drag { x1, y1, x2, y2 } => emit_response(cli.json, drag_response(x1, y1, x2, y2)),
        Commands::Do { steps } => {
            let response = match parse_batch_steps(&steps) {
                Ok(parsed_steps) => do_response(&parsed_steps),
                Err(message) => batch_error_response(message),
            };
            emit_batch_response(cli.json, response)
        }
    }
}

fn emit_response(json_mode: bool, response: Response) -> i32 {
    if json_mode {
        println!("{}", serde_json::to_string(&response.json).expect("serialize response"));
    } else if !response.text.is_empty() {
        println!("{}", response.text);
    }

    response.exit_code
}

fn emit_batch_response(json_mode: bool, response: BatchResponse) -> i32 {
    if json_mode {
        println!("{}", serde_json::to_string(&response.json).expect("serialize batch response"));
    } else {
        for line in response.lines {
            println!("{line}");
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

fn disconnect_response() -> Response {
    Response {
        text: "disconnected".to_string(),
        json: json!({
            "ok": true,
            "command": "disconnect",
            "was_connected": false
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

fn capture_response(file: &str, format: CaptureFormat, region: Option<Region>) -> Response {
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

    let json = if let Some(region) = region {
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

fn click_response(button: MouseButton, x: i32, y: i32) -> Response {
    Response {
        text: format!("clicked button={} x={x} y={y}", button.as_str()),
        json: json!({
            "ok": true,
            "command": "click",
            "button": button.as_str(),
            "x": x,
            "y": y
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
        "disconnect" => disconnect_response(),
        "status" => status_response(),
        "capture" => {
            let file = first_non_flag_arg(&step.args).unwrap_or("screenshot.png");
            let format = flag_value(&step.args, "--format")
                .and_then(parse_capture_format)
                .unwrap_or_else(|| infer_format(file));
            let region = flag_value(&step.args, "--region").and_then(|raw| raw.parse::<Region>().ok());
            capture_response(file, format, region)
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
            let coords = positional_args(&step.args);
            let x = coords.first().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            let y = coords.get(1).and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
            click_response(button, x, y)
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

fn parse_batch_steps(tokens: &[String]) -> Result<Vec<BatchStep>, String> {
    let mut index = 0;
    let mut steps = Vec::new();

    while index < tokens.len() {
        let command = tokens[index].clone();
        if !is_step_command(&command) {
            return Err(format!("unknown batch command '{command}'"));
        }

        match command.as_str() {
            "disconnect" | "status" => {
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
            "connect" => {
                let (args, next_index) = parse_flagged_step(tokens, index + 1, &["--password", "--server", "--timeout"], 1)?;
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
                    parse_flagged_step(tokens, index + 1, &["--format", "--quality", "--region"], 1)?;
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
        "connect" | "disconnect" | "status" | "capture" | "type" | "key" | "click" | "move" | "drag"
    )
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
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
