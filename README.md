# rustdesk-cli

Command-line RustDesk client for AI agents. Provides programmatic remote desktop control via RustDesk's protocol — connect, capture screenshots, send keyboard/mouse input, and query connection status.

## Usage

```
rustdesk-cli <COMMAND>

Commands:
  connect     Connect to a remote RustDesk peer
  disconnect  Disconnect from current peer
  capture     Capture a screenshot (PNG)
  type        Type text on the remote machine
  key         Send a key press (e.g. "enter", "ctrl+c")
  click       Click at (x, y) coordinates
  move        Move cursor to (x, y)
  status      Show connection status
```

## Examples

```bash
rustdesk-cli connect 123456789 --password secret
rustdesk-cli capture --output screen.png
rustdesk-cli type "hello world"
rustdesk-cli key enter
rustdesk-cli click 500 300 --button left
rustdesk-cli move 100 200
rustdesk-cli status
rustdesk-cli disconnect
```

## Building

```bash
cargo build
cargo build --release
```

## Architecture

- **Protocol**: RustDesk uses protobuf over an encrypted TCP connection (with rendezvous server for NAT traversal)
- **Dependencies**: prost for protobuf, tokio for async, image for PNG encoding, clap for CLI
- **Status**: Scaffold only — protocol implementation pending

## Notes

- `hbb_common` from the RustDesk repo is not vendored due to heavy build dependencies. We will extract and vendor the specific `.proto` definitions needed for the client protocol.
