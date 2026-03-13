# rustdesk-cli

Command-line RustDesk client for AI agents. Provides programmatic remote desktop control via RustDesk's protocol — connect, capture screenshots, send keyboard/mouse input, run commands, and manage sessions.

## Features

- Full RustDesk protocol: rendezvous discovery, relay fallback, NaCl encryption, password auth
- Screenshot capture with region cropping, format conversion (PNG/JPEG), quality control
- Keyboard input (type text, send key combos with modifiers)
- Mouse control (click, double-click, move, drag, scroll)
- Remote shell (interactive terminal, command execution)
- Clipboard get/set
- Multi-display support (list displays, capture specific display)
- JSON output mode for machine consumption (`--json`)
- Batch command execution (`do` subcommand)
- Daemon architecture with reconnection, keepalive, graceful shutdown
- Password via flag, stdin, or environment variable

## Usage

```
rustdesk-cli [OPTIONS] <COMMAND>

Commands:
  connect     Connect to a remote RustDesk peer
  disconnect  Disconnect from current peer
  status      Show connection status
  displays    List available displays on the remote peer
  capture     Capture a screenshot
  type        Type text on the remote machine
  key         Send a key press (e.g. "enter", "a --modifiers meta")
  click       Click at (x, y) coordinates
  scroll      Scroll at (x, y) by delta
  move        Move cursor to (x, y)
  drag        Drag from (x1, y1) to (x2, y2)
  shell       Open interactive remote shell
  exec        Execute a command on the remote machine
  clipboard   Get or set remote clipboard
  do          Execute multiple commands in sequence

Options:
  --json      Emit machine-readable JSON output
  --version   Print version
```

## Examples

```bash
# Connect with password
rustdesk-cli connect 123456789 --password secret

# Connect with password from stdin (for scripts)
echo "secret" | rustdesk-cli connect 123456789 --password-stdin

# Connect with password from environment
RUSTDESK_PASSWORD=secret rustdesk-cli connect 123456789

# Capture screenshot
rustdesk-cli capture screen.png
rustdesk-cli capture screen.jpg --format jpg --quality 90
rustdesk-cli capture --display 1 --region 100,200,300,400 region.png

# Pipe screenshot to stdout
rustdesk-cli capture > screen.png

# Input control
rustdesk-cli type "hello world"
rustdesk-cli key enter
rustdesk-cli key a --modifiers meta    # Cmd+A on macOS
rustdesk-cli click 500 300
rustdesk-cli click --double 500 300
rustdesk-cli scroll 500 300 3
rustdesk-cli move 100 200
rustdesk-cli drag 0 0 100 100

# Remote shell and commands
rustdesk-cli shell
rustdesk-cli exec --command "ls -la"

# Clipboard
rustdesk-cli clipboard get
rustdesk-cli clipboard set --text "hello"

# List remote displays
rustdesk-cli displays

# Batch execution
rustdesk-cli do capture shot.png click 500 300 type "done"

# JSON output (for AI agents)
rustdesk-cli --json status
rustdesk-cli --json capture screen.png --display 0

# Disconnect
rustdesk-cli disconnect
```

## Building

```bash
cargo build --release
```

## Testing

```bash
cargo test                          # Run all unit + integration tests
cargo test -- --ignored             # Run live server tests (requires config)
```

## Architecture

- **Protocol**: RustDesk rendezvous (hbbs) + relay (hbbr) + NaCl encrypted TCP
- **Daemon**: Background process manages persistent connection, handles reconnection with exponential backoff, session keepalive (45s timeout), signal handling (SIGTERM/SIGINT)
- **CLI ↔ Daemon**: Unix domain socket with JSON protocol
- **Dependencies**: prost (protobuf), tokio (async), image (capture), clap (CLI), NaCl crypto stack

## Server Configuration

Connect to self-hosted RustDesk servers:

```bash
rustdesk-cli connect PEER_ID --password PW \
  --id-server host:21116 \
  --relay-server host:21117 \
  --server-key BASE64_KEY
```
