# Architecture Pivot: Text-First CLI

## Discovery: RustDesk Has Native Terminal Support

The `message.proto` defines a complete terminal channel that we were NOT using:

```protobuf
// In the Message union (fields 31-32):
TerminalAction terminal_action = 31;
TerminalResponse terminal_response = 32;

// Terminal sub-messages:
message OpenTerminal   { int32 terminal_id; uint32 rows; uint32 cols; }
message TerminalData   { int32 terminal_id; bytes data; bool compressed; }
message ResizeTerminal { int32 terminal_id; uint32 rows; uint32 cols; }
message CloseTerminal  { int32 terminal_id; }

message TerminalOpened { int32 terminal_id; bool success; string message; uint32 pid; ... }
message TerminalData   { int32 terminal_id; bytes data; bool compressed; }  // stdout
message TerminalClosed { int32 terminal_id; int32 exit_code; }
message TerminalError  { int32 terminal_id; string message; }
```

This is a **native remote shell** — raw PTY I/O, no video decoding needed.

## What This Means

### We DON'T need (for text-mode):
- VP9/H264/AV1 video decoding
- Video frame processing pipeline
- VideoFrame message handling
- Display coordinate system (x,y click/drag/scroll)
- Mouse events
- Cursor tracking
- Audio frames
- Screenshot decoding (though `ScreenshotRequest/Response` is useful for occasional visual checks)

### We DO need (priority order):
1. **TerminalAction/TerminalResponse** — the primary text channel
   - `OpenTerminal` → open a shell (specify rows/cols)
   - `TerminalData` → bidirectional stdin/stdout bytes
   - `ResizeTerminal` → PTY resize (SIGWINCH)
   - `CloseTerminal` → close session
2. **Clipboard** — for bulk text transfer
   - `Clipboard { content: bytes, format: ClipboardFormat }` — sync clipboard text
   - `MultiClipboards` — multiple clipboard items
3. **ScreenshotRequest/ScreenshotResponse** — on-demand visual check (no video stream)
   - Returns PNG/image data in `ScreenshotResponse.data`
   - Useful for AI agents that occasionally need to "see" the screen
4. **Misc::ChatMessage** — simple text messaging channel
5. **FileAction/FileResponse** — file transfer (useful for deploying scripts)

### Architecture Changes

**Current flow** (over-engineered for our use case):
```
connect → relay → crypto → auth → video_frame loop → decode VP9 → screenshot
```

**New flow** (text-first):
```
connect → relay → crypto → auth → OpenTerminal → TerminalData bidirectional stream
```

### CLI Commands (Revised)

| Command | Priority | Channel | Notes |
|---------|----------|---------|-------|
| `connect` | P0 | Rendezvous+Relay+Auth | Same as now |
| `shell` | P0 | TerminalAction | Interactive PTY session |
| `exec <cmd>` | P0 | TerminalAction | Run command, return output |
| `clipboard get/set` | P1 | Clipboard | Bulk text transfer |
| `screenshot` | P1 | ScreenshotRequest | On-demand PNG, no video stream |
| `upload/download` | P2 | FileAction | File transfer |
| `type/key/click` | P3 | KeyEvent/MouseEvent | Keep for GUI fallback |
| `capture` (video) | P3 | VideoFrame | Deprioritize |

### What to Simplify

1. **Remove `VideoFrame` from `protocol.rs`** placeholder types (unused, will use proto)
2. **Remove `VideoCodecFormat`** from protocol.rs (not needed for text mode)
3. **Skip video codec dependencies** entirely — no VP9/H264 decoder
4. **Simplify `LoginRequest`** — set `video_ack_required: false`, skip codec negotiation
5. **Focus `session.rs` commands** on terminal + clipboard first

### ConnType

The relay `RequestRelay` has a `conn_type` field:
- `DEFAULT_CONN = 0` — full desktop (video + input)
- `FILE_TRANSFER = 1` — file transfer only
- `PORT_FORWARD = 2` — TCP port forwarding

For terminal, we likely use `DEFAULT_CONN` (terminal is a feature within a default connection,
not a separate connection type). After auth, we send `OpenTerminal` instead of waiting for video frames.

### OptionMessage

After login, send `OptionMessage` with:
- `disable_audio: Yes` — we don't need audio
- `image_quality: Low` or skip entirely — we're not rendering video
- The server may still send video frames; we simply ignore them
- `terminal_persistent: Yes` — reconnect to existing terminal sessions
