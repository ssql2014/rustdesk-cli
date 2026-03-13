# rustdesk-cli Design

## Goals

`rustdesk-cli` is a minimal command-line RustDesk client for AI agents. The design should:

- Keep the command set small and predictable.
- Preserve one active remote session across invocations.
- Make every command usable in either human-readable text mode or machine-readable JSON mode.
- Avoid hidden state where possible, especially for pointer operations.

## Global Conventions

### Top-level syntax

```text
rustdesk-cli [--json] <command> [args...]
rustdesk-cli [--json] do <step...>
```

### Session model

- `connect` creates or replaces the single active local session.
- `status`, `capture`, `type`, `key`, `click`, `move`, and `drag` operate on that active session.
- `disconnect` is idempotent. If no session exists, it still exits successfully.
- `do` runs all steps in one process and uses the same active session context as normal commands.

### Output rules

- Text mode is the default.
- `--json` is a global flag and always prints exactly one JSON object to `stdout`.
- Success output goes to `stdout`.
- Errors and diagnostics go to `stderr`.
- Sensitive values are redacted from output. `type` never echoes the full typed string, and `connect` never echoes the password.

### Exit codes

Stable runtime exit codes:

- `0`: success
- `1`: connection error
- `2`: authentication error
- `3`: timeout

Argument parsing and local file errors may use standard non-runtime exit codes from the CLI framework; the agent-facing contract above is for transport/runtime outcomes.

### Common data rules

- Coordinates are integer remote-screen pixels with origin at top-left.
- Regions use `x,y,w,h`.
- `--timeout` is in seconds.
- File paths are local paths on the machine running `rustdesk-cli`.
- All JSON examples below are representative; field order is not significant.

## Command Reference

### 1. Connection Management

#### `connect`

```text
rustdesk-cli connect <id> [--password <pw>] [--server <addr>] [--timeout <sec>]
```

- `<id>`: RustDesk remote ID, kept as a string.
- `--password <pw>`: connection password. If `<pw>` is `-`, read one line from `stdin`.
- `--server <addr>`: optional RustDesk rendezvous/relay override, for example `rs.example.com:21116`.
- `--timeout <sec>`: default `15`.

Text output:

```text
connected id=123456789 server=rs.example.com width=1920 height=1080
```

JSON output:

```json
{
  "ok": true,
  "command": "connect",
  "id": "123456789",
  "server": "rs.example.com:21116",
  "connected": true,
  "width": 1920,
  "height": 1080
}
```

Failure behavior:

- Wrong password: exit `2`
- Timeout while establishing session: exit `3`
- Network/session failure: exit `1`

#### `disconnect`

```text
rustdesk-cli disconnect
```

Text output:

```text
disconnected
```

JSON output:

```json
{
  "ok": true,
  "command": "disconnect",
  "was_connected": true
}
```

Notes:

- If already disconnected, still return `0`.
- In JSON mode, `was_connected` is `false` when there was no active session.

#### `status`

```text
rustdesk-cli status
```

Text output when connected:

```text
connected id=123456789 server=rs.example.com width=1920 height=1080
```

Text output when disconnected:

```text
disconnected
```

JSON output when connected:

```json
{
  "ok": true,
  "command": "status",
  "connected": true,
  "id": "123456789",
  "server": "rs.example.com:21116",
  "width": 1920,
  "height": 1080
}
```

JSON output when disconnected:

```json
{
  "ok": true,
  "command": "status",
  "connected": false
}
```

Notes:

- `status` returns `0` whether connected or not. Disconnected is a valid state, not an error.

### 2. Screen Capture

#### `capture`

```text
rustdesk-cli capture <file> [--format png|jpg] [--quality N] [--region x,y,w,h]
```

- `<file>`: output image path.
- `--format`: default is inferred from file extension, otherwise `png`.
- `--quality N`: JPEG quality `1-100`, default `90`. Ignored for PNG.
- `--region x,y,w,h`: optional crop rectangle in remote-screen coordinates.

Text output:

```text
captured file=shot.png format=png width=1920 height=1080 bytes=482193
```

Text output with region:

```text
captured file=dialog.jpg format=jpg width=640 height=480 bytes=88421 region=100,120,640,480
```

JSON output:

```json
{
  "ok": true,
  "command": "capture",
  "file": "shot.png",
  "format": "png",
  "width": 1920,
  "height": 1080,
  "bytes": 482193
}
```

JSON output with region:

```json
{
  "ok": true,
  "command": "capture",
  "file": "dialog.jpg",
  "format": "jpg",
  "width": 640,
  "height": 480,
  "bytes": 88421,
  "region": {
    "x": 100,
    "y": 120,
    "w": 640,
    "h": 480
  }
}
```

Failure behavior:

- No active session: exit `1`
- Remote capture timeout: exit `3`

### 3. Input Control

#### `type`

```text
rustdesk-cli type "text"
```

- Sends the literal UTF-8 string to the remote host.
- Does not append Enter automatically. Use `key enter` when needed.

Text output:

```text
typed chars=5
```

JSON output:

```json
{
  "ok": true,
  "command": "type",
  "chars": 5,
  "redacted": true
}
```

Failure behavior:

- No active session: exit `1`

#### `key`

```text
rustdesk-cli key <keyname> [--modifiers ctrl,shift,alt]
```

- `<keyname>` is lowercase and stable, for example `enter`, `tab`, `esc`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1` to `f12`, `a`, `1`.
- `--modifiers` is a comma-separated list with no spaces.
- A `key` command sends one combined press-and-release chord.

Text output:

```text
key key=enter
```

With modifiers:

```text
key key=a modifiers=ctrl,shift
```

JSON output:

```json
{
  "ok": true,
  "command": "key",
  "key": "a",
  "modifiers": ["ctrl", "shift"]
}
```

Failure behavior:

- No active session: exit `1`

#### `click`

```text
rustdesk-cli click [--button left|right|middle] <x> <y>
```

- Default button is `left`.
- `click` both moves to the target and clicks there.
- This is intentionally self-contained so agents do not depend on prior pointer state.

Text output:

```text
clicked button=left x=500 y=300
```

JSON output:

```json
{
  "ok": true,
  "command": "click",
  "button": "left",
  "x": 500,
  "y": 300
}
```

Failure behavior:

- No active session: exit `1`

#### `move`

```text
rustdesk-cli move <x> <y>
```

Text output:

```text
moved x=500 y=300
```

JSON output:

```json
{
  "ok": true,
  "command": "move",
  "x": 500,
  "y": 300
}
```

Failure behavior:

- No active session: exit `1`

#### `drag`

```text
rustdesk-cli drag <x1> <y1> <x2> <y2>
```

- Performs: move to `x1,y1`, press left button, move to `x2,y2`, release.
- Left-button drag only in v1. This keeps the API minimal.

Text output:

```text
dragged x1=100 y1=100 x2=400 y2=100
```

JSON output:

```json
{
  "ok": true,
  "command": "drag",
  "x1": 100,
  "y1": 100,
  "x2": 400,
  "y2": 100,
  "button": "left"
}
```

Failure behavior:

- No active session: exit `1`

### 4. Batch Mode

#### `do`

```text
rustdesk-cli do connect 123456 --password pw click 500 300 type "hello" key enter capture shot.png
```

Allowed step verbs:

- `connect`
- `disconnect`
- `status`
- `capture`
- `type`
- `key`
- `click`
- `move`
- `drag`

Parsing rules:

- `do` consumes a sequence of normal subcommands without repeating `rustdesk-cli`.
- Each step starts when the parser sees the next known verb token.
- Shell quoting still applies before `rustdesk-cli` sees the arguments.

Execution rules:

- Steps run strictly in order.
- The first failing step stops the batch.
- The process exit code is the failing step's exit code.
- If all steps succeed, `do` exits `0`.
- Session changes persist exactly as if the commands had been run separately.

Text output:

```text
1 connected id=123456 server=rs.example.com width=1920 height=1080
2 clicked button=left x=500 y=300
3 typed chars=5
4 key key=enter
5 captured file=shot.png format=png width=1920 height=1080 bytes=482193
ok steps=5
```

JSON output:

```json
{
  "ok": true,
  "command": "do",
  "steps": [
    {
      "index": 1,
      "command": "connect",
      "ok": true,
      "id": "123456",
      "server": "rs.example.com:21116",
      "width": 1920,
      "height": 1080
    },
    {
      "index": 2,
      "command": "click",
      "ok": true,
      "button": "left",
      "x": 500,
      "y": 300
    },
    {
      "index": 3,
      "command": "type",
      "ok": true,
      "chars": 5,
      "redacted": true
    },
    {
      "index": 4,
      "command": "key",
      "ok": true,
      "key": "enter",
      "modifiers": []
    },
    {
      "index": 5,
      "command": "capture",
      "ok": true,
      "file": "shot.png",
      "format": "png",
      "width": 1920,
      "height": 1080,
      "bytes": 482193
    }
  ]
}
```

JSON output on failure:

```json
{
  "ok": false,
  "command": "do",
  "failed_step": 3,
  "error": {
    "code": "connection_error",
    "message": "remote session lost"
  },
  "steps": [
    {
      "index": 1,
      "command": "connect",
      "ok": true
    },
    {
      "index": 2,
      "command": "click",
      "ok": true
    },
    {
      "index": 3,
      "command": "type",
      "ok": false,
      "error": {
        "code": "connection_error",
        "message": "remote session lost"
      }
    }
  ]
}
```

## Error Output Shape

Text mode error example:

```text
connection error: no active session
```

JSON mode error example:

```json
{
  "ok": false,
  "command": "capture",
  "error": {
    "code": "connection_error",
    "message": "no active session"
  }
}
```

Recommended stable error codes inside JSON:

- `connection_error`
- `auth_error`
- `timeout`
- `invalid_region`
- `unsupported_key`
- `io_error`

## Comparison with vncdotool

The main design choice is to stay close to `vncdotool` where it helps muscle memory, but remove pointer-state ambiguity for agents.

| Task | `rustdesk-cli` | `vncdotool` equivalent |
|------|----------------|------------------------|
| Connect to remote target | `rustdesk-cli connect 123456 --password pw` | No exact equivalent; `vncdotool` usually connects per invocation with `-s host::port` |
| Disconnect | `rustdesk-cli disconnect` | End the `vncdotool` process or connection context |
| Check connection state | `rustdesk-cli status` | No close single-command equivalent; usually implicit in whether the command connects |
| Type text | `rustdesk-cli type "hello"` | `vncdotool type "hello"` |
| Press Enter | `rustdesk-cli key enter` | `vncdotool key enter` |
| Press Ctrl+A | `rustdesk-cli key a --modifiers ctrl` | `vncdotool key ctrl-a` |
| Move pointer | `rustdesk-cli move 500 300` | `vncdotool move 500 300` |
| Click at coordinates | `rustdesk-cli click 500 300` | `vncdotool move 500 300 click 1` |
| Right-click at coordinates | `rustdesk-cli click --button right 500 300` | `vncdotool move 500 300 click 3` |
| Drag from A to B | `rustdesk-cli drag 100 100 400 100` | Typically `vncdotool move 100 100 drag 400 100` |
| Capture full screen | `rustdesk-cli capture shot.png` | `vncdotool capture shot.png` |
| Capture region | `rustdesk-cli capture shot.png --region 100,100,800,600` | `vncdotool rcapture shot.png 100 100 800 600` |
| Batch several actions | `rustdesk-cli do connect 123456 click 500 300 type "hello" key enter capture shot.png` | `vncdotool -s host::port move 500 300 click 1 type "hello" key enter capture shot.png` |

## Recommended UX Summary

- Keep a single active session so normal commands stay short.
- Make `click` coordinate-based to remove dependence on prior `move`.
- Always give screenshot metadata on `stdout` after capture.
- Make `--json` global and uniform across every command.
- Make `do` stop on first error and return partial step results in JSON mode.
