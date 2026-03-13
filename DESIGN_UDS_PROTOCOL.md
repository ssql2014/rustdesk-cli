# DESIGN_UDS_PROTOCOL

## Purpose

Define the exact wire protocol between the `rustdesk-cli` CLI process and the local daemon over the Unix domain socket.

This protocol must support:

- simple request/response commands
- structured errors
- interactive shell bidirectional streaming over the same socket
- future extension without breaking old clients

This document is concrete and implementable. It is the source of truth for local IPC behavior.

## Scope

In scope:

- frame format
- protocol version negotiation
- request types
- response types
- shell streaming multiplexing on a single UDS connection
- backpressure behavior

Out of scope:

- RustDesk network protocol
- remote host message formats
- CLI text rendering

## Design Summary

The UDS protocol will use:

- a binary length-prefixed frame format
- UTF-8 JSON payloads for control-plane messages
- raw byte payloads for shell stdin/stdout stream frames

Rationale:

- newline-delimited JSON is not safe for raw terminal byte streams
- protobuf is unnecessary for local IPC and would complicate fast iteration
- a binary frame header plus typed payload keeps control messages readable and stream messages efficient

## 1. Transport Model

## 1.1 Socket Type

- Unix domain stream socket
- one client connection per top-level CLI invocation
- request/response commands use short-lived connections
- shell uses a long-lived full-duplex connection

## 1.2 Endianness

All integer fields in the frame header are little-endian.

## 1.3 Connection Lifecycle

Every client connection begins with protocol negotiation:

1. client connects to UDS
2. client sends `Hello`
3. daemon sends `HelloAck`
4. then normal frames flow

No application requests may be sent before successful `HelloAck`.

## 2. Frame Format

## 2.1 Header

Every frame has a fixed 16-byte header followed by `payload_len` bytes of payload.

Header layout:

```text
offset  size  field
0       4     magic
4       2     version
6       2     frame_type
8       4     flags
12      4     payload_len
```

Field definitions:

- `magic`
  - ASCII `RDCX`
  - bytes: `0x52 0x44 0x43 0x58`
- `version`
  - protocol version for this frame
  - initial version: `1`
- `frame_type`
  - enum defined below
- `flags`
  - bitfield, semantics depend on frame type
  - unset bits must be `0`
- `payload_len`
  - unsigned payload size in bytes
  - may be `0`

Maximum payload:

- default max frame payload: 1 MiB
- frames larger than this must be rejected with protocol error

Rationale:

- enough for terminal chunks and JSON control messages
- small enough to prevent unbounded local memory abuse

## 2.2 Frame Type Registry

Frame type values:

```text
0x0001 Hello
0x0002 HelloAck
0x0003 Request
0x0004 ResponseSuccess
0x0005 ResponseError
0x0006 StreamData
0x0007 StreamWindow
0x0008 StreamEnd
0x0009 Ping
0x000A Pong
```

## 2.3 Payload Encoding By Frame Type

- `Hello`
  - JSON
- `HelloAck`
  - JSON
- `Request`
  - JSON
- `ResponseSuccess`
  - JSON
- `ResponseError`
  - JSON
- `StreamData`
  - raw bytes
- `StreamWindow`
  - JSON
- `StreamEnd`
  - JSON
- `Ping`
  - empty payload
- `Pong`
  - empty payload

## 2.4 Stream Identifier

There is at most one logical shell stream per UDS connection in protocol version 1.

Therefore:

- `StreamData`
- `StreamWindow`
- `StreamEnd`

do not carry a separate stream id in v1.

If multiplexed local substreams are needed later, they require protocol version 2.

## 3. Version Negotiation

## 3.1 Hello

Client must send `Hello` immediately after connect.

`Hello` JSON payload:

```json
{
  "client_protocol_version": 1,
  "client_name": "rustdesk-cli",
  "client_pid": 12345,
  "mode": "command"
}
```

`mode` values:

- `command`
- `shell`

`mode` is advisory and used for logging/debugging only in v1.

## 3.2 HelloAck

Daemon replies with `HelloAck`.

Successful `HelloAck` payload:

```json
{
  "ok": true,
  "daemon_protocol_version": 1,
  "daemon_pid": 67890,
  "features": [
    "request_response",
    "shell_stream_v1",
    "stream_window_v1"
  ]
}
```

Rejected version payload:

```json
{
  "ok": false,
  "error": {
    "kind": "input_error",
    "message": "unsupported protocol version"
  },
  "supported_versions": [1]
}
```

If `HelloAck.ok` is false, the connection must be closed by both sides.

## 3.3 Version Rule

In v1:

- client sends exactly one version number
- daemon either accepts it or rejects it
- no range negotiation is required

This is sufficient because the CLI and daemon are expected to come from the same installation.

## 4. Control-Plane Messages

## 4.1 Request Frame

All non-streaming commands are sent in a `Request` frame with JSON payload.

Common fields:

```json
{
  "request_id": "uuid-or-monotonic-string",
  "command": "Status",
  "args": {}
}
```

Rules:

- `request_id` is client-generated and unique per connection
- daemon must echo `request_id` in responses
- `command` is a string enum
- `args` is a command-specific JSON object

## 4.2 Supported Request Types

### Connect

```json
{
  "request_id": "r1",
  "command": "Connect",
  "args": {
    "peer_id": "308235080",
    "password": "secret",
    "server": "1.2.3.4:21116",
    "id_server": "1.2.3.4:21116",
    "relay_server": "1.2.3.4:21117",
    "key": "base64-server-key"
  }
}
```

### Disconnect

```json
{
  "request_id": "r2",
  "command": "Disconnect",
  "args": {}
}
```

### Shell

`Shell` is a control request that upgrades the same socket into streaming mode after `ResponseSuccess`.

```json
{
  "request_id": "r3",
  "command": "Shell",
  "args": {
    "rows": 24,
    "cols": 80
  }
}
```

### Exec

```json
{
  "request_id": "r4",
  "command": "Exec",
  "args": {
    "command": "echo hello"
  }
}
```

### ClipboardGet

```json
{
  "request_id": "r5",
  "command": "ClipboardGet",
  "args": {}
}
```

### ClipboardSet

```json
{
  "request_id": "r6",
  "command": "ClipboardSet",
  "args": {
    "text": "hello"
  }
}
```

### Status

```json
{
  "request_id": "r7",
  "command": "Status",
  "args": {}
}
```

### Capture

```json
{
  "request_id": "r8",
  "command": "Capture",
  "args": {
    "output": "shot.png"
  }
}
```

## 4.3 Unsupported In v1

The following are not part of the UDS protocol v1 request set:

- batch `Do`
  - batch stays a CLI-side orchestration concern in v1 unless a future daemon batch executor is introduced
- shell reattach
- multiple parallel shell streams on one UDS connection

## 5. Response Types

## 5.1 ResponseSuccess

Used for successful completion of a control request.

JSON payload:

```json
{
  "request_id": "r7",
  "ok": true,
  "command": "Status",
  "data": {
    "connected": true,
    "id": "308235080"
  }
}
```

Rules:

- exactly one terminal `ResponseSuccess` or `ResponseError` must be sent for every non-shell request
- `request_id` must match the initiating request

## 5.2 ResponseError

Used for failed control requests.

JSON payload:

```json
{
  "request_id": "r7",
  "ok": false,
  "command": "Status",
  "error": {
    "kind": "session_error",
    "message": "No active session"
  }
}
```

Required error fields:

- `kind`
  - `connection_error`
  - `session_error`
  - `input_error`
  - `internal_error`
- `message`

Optional error fields:

- `code`
- `details`

## 5.3 Shell Success Response

For `Shell`, `ResponseSuccess` is an attach/upgrade ack, not the end of the command.

Example:

```json
{
  "request_id": "r3",
  "ok": true,
  "command": "Shell",
  "data": {
    "mode": "stream",
    "terminal_id": 1,
    "rows": 24,
    "cols": 80,
    "stream_protocol": "shell_stream_v1"
  }
}
```

After this frame is sent, both sides switch to streaming behavior on the same socket.

## 6. Shell Streaming Protocol

## 6.1 Upgrade Rule

Shell uses the same socket as the initial request/ack.

Flow:

1. `Hello`
2. `HelloAck`
3. `Request(command=Shell)`
4. `ResponseSuccess(command=Shell, mode=stream)`
5. zero or more stream frames
6. `StreamEnd`
7. socket close by either side

After step 4:

- no further `Request` frames are allowed on that connection
- the connection is dedicated to shell streaming only

This keeps protocol state simple and avoids multiplexing command/stream traffic on one live shell socket.

## 6.2 StreamData Semantics

`StreamData` direction determines meaning:

- CLI -> daemon
  - stdin bytes for remote PTY
- daemon -> CLI
  - stdout/stderr bytes from remote PTY

Header usage:

- `frame_type = StreamData`
- `flags` bit 0
  - `0` = stdin direction
  - `1` = stdout direction

Payload:

- raw uninterpreted bytes

Rules:

- payload may be zero-length only if used as a no-op keepalive, though this is discouraged
- no UTF-8 requirement
- no newline requirement

## 6.3 StreamWindow Semantics

Used for resize and backpressure window updates.

JSON payload includes a `kind` field.

### Resize

CLI -> daemon:

```json
{
  "kind": "resize",
  "rows": 40,
  "cols": 120
}
```

Daemon converts this to `ResizeTerminal`.

### Credit Update

Either direction may send:

```json
{
  "kind": "credit",
  "bytes": 65536
}
```

This is the protocol’s explicit backpressure signal.

## 6.4 StreamEnd

Used to end shell streaming cleanly.

JSON payload:

```json
{
  "reason": "remote_closed",
  "exit_code": 0
}
```

Allowed `reason` values:

- `eof`
- `local_close`
- `remote_closed`
- `disconnect`
- `transport_error`
- `protocol_error`
- `session_error`

Rules:

- once `StreamEnd` is sent, no more `StreamData` may be sent
- sender should flush any already-buffered frames before `StreamEnd`
- receiver should treat `StreamEnd` as terminal and move to cleanup

## 6.5 Error During Streaming

Streaming errors are signaled with `StreamEnd`, not `ResponseError`.

Rationale:

- after shell upgrade, the socket is no longer in request/response mode

Example:

```json
{
  "reason": "transport_error",
  "error": {
    "kind": "connection_error",
    "message": "remote terminal transport closed"
  }
}
```

## 7. Backpressure

## 7.1 Problem

Shell output may flood the UDS if:

- the remote PTY emits large output bursts
- the CLI local terminal writes slowly
- the CLI is suspended or backpressured by the terminal

The protocol needs explicit behavior so the daemon does not buffer unbounded shell output in memory.

## 7.2 Credit-Based Flow Control

Protocol v1 will use receiver-advertised byte credit for shell output.

Rules:

- daemon may send stdout `StreamData` only while it has positive CLI-advertised credit
- CLI initially grants credit after shell upgrade
- CLI replenishes credit as it consumes and writes bytes locally

Suggested defaults:

- initial CLI credit: 65536 bytes
- replenish when remaining credit falls below half window
- replenish in chunks, not per write

This is simple, local, and sufficient for one shell stream.

## 7.3 Credit Accounting

Only daemon -> CLI stdout traffic is credit-limited in v1.

CLI -> daemon stdin traffic:

- relies on OS socket backpressure plus bounded client-side input queue
- no explicit remote stdin credit in v1

Rationale:

- stdin traffic is usually tiny
- stdout flooding is the real risk

## 7.4 Overflow Behavior

If daemon receives terminal output while CLI credit is exhausted:

- buffer only up to a bounded daemon-side shell output limit
- if that limit is exceeded, end stream with:
  - `reason = "transport_error"`
  - error kind `session_error` or `internal_error` depending on cause

Do not allow unbounded accumulation.

## 7.5 CLI Output Sink Behavior

CLI shell output loop should:

1. read `StreamData`
2. write bytes to local stdout
3. decrement credit
4. replenish via `StreamWindow(kind=credit)` after successful local write

Credit should reflect bytes successfully handed to the local TTY, not merely bytes received from UDS.

## 8. Concrete Message Catalog

## 8.1 Hello

Frame type: `Hello`

Payload:

```json
{
  "client_protocol_version": 1,
  "client_name": "rustdesk-cli",
  "client_pid": 12345,
  "mode": "command"
}
```

## 8.2 HelloAck

Frame type: `HelloAck`

Payload:

```json
{
  "ok": true,
  "daemon_protocol_version": 1,
  "daemon_pid": 67890,
  "features": [
    "request_response",
    "shell_stream_v1",
    "stream_window_v1"
  ]
}
```

## 8.3 Connect Request / ResponseSuccess

Request:

```json
{
  "request_id": "r1",
  "command": "Connect",
  "args": {
    "peer_id": "308235080",
    "password": "secret",
    "server": "1.2.3.4:21116",
    "id_server": "1.2.3.4:21116",
    "relay_server": "1.2.3.4:21117",
    "key": "server-key"
  }
}
```

Success:

```json
{
  "request_id": "r1",
  "ok": true,
  "command": "Connect",
  "data": {
    "connected": true,
    "id": "308235080"
  }
}
```

## 8.4 Disconnect ResponseError Example

```json
{
  "request_id": "r2",
  "ok": false,
  "command": "Disconnect",
  "error": {
    "kind": "session_error",
    "message": "No active session"
  }
}
```

## 8.5 Shell Stream Example

Handshake:

```json
{
  "request_id": "r3",
  "command": "Shell",
  "args": {
    "rows": 24,
    "cols": 80
  }
}
```

Ack:

```json
{
  "request_id": "r3",
  "ok": true,
  "command": "Shell",
  "data": {
    "mode": "stream",
    "terminal_id": 1,
    "rows": 24,
    "cols": 80,
    "stream_protocol": "shell_stream_v1"
  }
}
```

Then:

- CLI sends `StreamWindow(kind=credit, bytes=65536)`
- CLI sends `StreamData(stdin)`
- daemon sends `StreamData(stdout)`
- CLI sends `StreamWindow(kind=resize, rows=40, cols=120)`
- daemon sends `StreamEnd(reason=remote_closed, exit_code=0)`

## 9. Daemon Implementation Rules

## 9.1 Request/Response Connections

For non-shell commands:

- exactly one request per socket
- exactly one terminal response frame
- daemon closes the socket after response flush

## 9.2 Shell Connections

For shell:

- exactly one `Shell` request per socket
- socket becomes dedicated shell stream after successful ack
- daemon must associate the socket with one interactive terminal runtime

## 9.3 Protocol Errors

The daemon must close the connection if:

- header magic is invalid
- payload length exceeds max
- frame type is unknown
- unexpected frame type appears in current state
- malformed JSON payload for control frames
- client sends a second request after shell upgrade

If possible:

- send `ResponseError` before close in request/response state
- send `StreamEnd(reason=protocol_error)` before close in shell streaming state

## 10. CLI Implementation Rules

## 10.1 Request/Response Mode

CLI behavior:

- connect
- send `Hello`
- wait for `HelloAck`
- send `Request`
- wait for `ResponseSuccess` or `ResponseError`
- close

## 10.2 Shell Mode

CLI behavior:

- connect
- send `Hello(mode=shell)`
- wait for `HelloAck`
- send `Request(command=Shell)`
- wait for `ResponseSuccess(command=Shell, mode=stream)`
- send initial credit
- enter concurrent read/write shell loops

## 10.3 Stream Shutdown

CLI must treat receipt of `StreamEnd` as authoritative shell termination and begin cleanup immediately.

## 11. Why This Design

This protocol chooses:

- binary frames instead of newline JSON
  - needed for raw shell bytes
- JSON control payloads instead of protobuf IPC
  - easier debugging and faster iteration locally
- explicit shell upgrade on same socket
  - satisfies bidirectional shell requirement without a second connection
- one shell stream per socket
  - keeps implementation simple for v1
- credit-based stdout flow control
  - prevents daemon memory blowups when shell output floods the local client

## 12. Final Contract

Protocol v1 contract:

- all UDS communication is framed with a 16-byte binary header
- control-plane messages use JSON payloads
- shell data-plane messages use raw byte payloads
- every connection starts with `Hello` / `HelloAck`
- non-shell commands are one request and one terminal response
- shell upgrades the same socket into streaming mode after `ResponseSuccess`
- shell stream uses:
  - `StreamData`
  - `StreamWindow`
  - `StreamEnd`
- daemon stdout streaming is credit-limited
- protocol version is explicitly negotiated

This is concrete enough for implementation in both CLI and daemon without inventing further local IPC semantics.
