# Post-Login Protocol Research: RustDesk

This document outlines the protocol flow and message exchange that occurs after a successful `LoginResponse` with `PeerInfo` in the RustDesk protocol.

## 1. Initial Exchange after LoginResponse

Once the client receives `LoginResponse(PeerInfo)`, it enters the authorized state.
- **PeerInfo Analysis:** The client examines `pi.features` (for terminal support) and `pi.platform_additions` (for installation status, Wayland, etc.).
- **Initialization:**
    - **Default Mode:** Starts clipboard synchronization service.
    - **File Transfer Mode:** Loads pending/last jobs.
    - **Terminal Mode:** Checks for terminal support and prepares the terminal UI.

## 2. Remote Desktop & Screen Sharing

Screen sharing is initiated by the "controlled" side (server) sending `VideoFrame` messages.
- **Trigger:** Typically triggered by the client being in "Default" connection mode.
- **Codecs:** Supports VP8, VP9, AV1, H264, and H265.
- **Flow:**
    1. Server starts a `video_service`.
    2. Server sends a `VideoFrame` (Protobuf) containing encoded data, display ID, and keyframe indicator.
    3. Client receives `VideoFrame`, determines the codec, and starts a decoding thread for that display ID.
    4. Client sends `Misc(ToggleVirtualDisplay)` or `Misc(TogglePrivacyMode)` if requested.

## 3. File Transfer Mechanism

File transfer uses `FileAction` and `FileResponse` messages.
- **Read/Write Jobs:** Managed by `TransferJob` in `hbb_common`.
- **Flow:**
    1. Client sends `FileAction(ReadDir)` to browse.
    2. Server responds with `FileResponse(Dir)` containing a list of `FileEntry`.
    3. To send a file, Client sends `FileAction(SendRequest)`.
    4. Server responds with `FileResponse(Digest)` for resume/overwrite check.
    5. Data is sent in `FileResponse(Block)` messages (chunked).
- **Chunking:** Handled by `FileTransferBlock` which includes a `blk_id` and compressed `data`.

## 4. Clipboard Synchronization

- **Messages:** `Clipboard` (for single format/text) and `MultiClipboards` (for multiple formats like images + text).
- **Format:** Supports Text, RTF, HTML, and RGBA images.
- **Flow:**
    1. Both sides monitor local clipboard changes.
    2. When a change occurs, the side sends a `Clipboard` message with compressed content.
    3. On macOS/Windows, specialized `Cliprdr` messages are used for advanced file copy-paste.

## 5. Command-Line Terminal Session (--terminal)

The terminal session is a specialized connection type (`ConnType::TERMINAL`).

- **Initiation:**
    - Client sends `LoginRequest` with `union: Terminal`.
    - After `LoginResponse`, Client sends `TerminalAction(OpenTerminal)` containing `terminal_id`, `rows`, and `cols`.
- **Server-Side Handling:**
    - Server receives `OpenTerminal`.
    - On Linux/macOS: Spawns a shell (e.g., `/bin/sh` or `/bin/bash`) using a PTY (Pseudo-Terminal).
    - On Windows: Uses a "helper process" pattern to handle `ConPTY` and user impersonation.
    - Server responds with `TerminalResponse(Opened)` including `pid` and `service_id`.
- **Data Streaming:**
    - **Input (Client to Server):** Client sends `TerminalAction(Data)` containing raw keystrokes/bytes in a `TerminalData` message.
    - **Output (Server to Client):** Server sends `TerminalResponse(Data)` containing raw bytes from the PTY.
- **Resizing:** Client sends `TerminalAction(ResizeTerminal)` to update the PTY dimensions.
- **Lifecycle:** Persistent sessions are supported via `service_id`. Reconnecting to an existing terminal uses the same ID.

## Conclusion for rustdesk-cli

For our CLI tool, the most critical flow is:
1. `LoginRequest(Terminal)`
2. Wait for `LoginResponse`
3. Send `TerminalAction(OpenTerminal)`
4. Pipe local STDIN to `TerminalAction(Data)`
5. Pipe `TerminalResponse(Data)` to local STDOUT.
