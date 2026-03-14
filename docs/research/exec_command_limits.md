# Research: Exec Command Size and Length Limitations

This document investigates the limitations encountered when sending large payloads via `rustdesk-cli exec`, specifically addressing the "daemon disconnect" issue reported with ~4KB payloads.

## 1. Protocol and Message Limits

Analysis of the RustDesk protocol and our implementation reveals no inherent small limit at the message level.

- **BytesCodec:** The framing layer supports up to **1GB** payloads.
- **Safety Limits:** Both the official client and `rustdesk-cli` use a **64MiB** safety cap for incoming messages.
- **Protobuf:** `TerminalData` uses a `bytes` field for the payload, which has no specific size restriction beyond available memory.
- **Compression:** `rustdesk-cli` compresses payloads > 1024 bytes using Zstd (level 3), which reduces the wire size for base64 or text data.

## 2. The PTY `MAX_INPUT` Bottleneck

The most significant limitation is not the protocol, but the **PTY (Pseudo-Terminal) line discipline** on the remote host.

- **Canonical Mode:** Most shells (`bash`, `sh`, `zsh`) operate in canonical (line-buffered) mode when waiting for a command.
- **MAX_INPUT:** On Linux and macOS, the PTY input buffer is typically limited to **4096 bytes**.
- **The Failure Case:** If you send a command string (like a large base64 blob inside a `cat` heredoc) that exceeds 4096 bytes without a newline, the PTY buffer fills up. Subsequent bytes (including the essential `\n` to execute the command) may be discarded or cause the writer to block.
- **Result:** The shell never "sees" the end of the line, the command is never executed, and the client hangs waiting for output or a sentinel.

## 3. Daemon Architectural Issues

The "disconnect" behavior is likely a side effect of how our daemon handles heartbeats.

- **Sequential Execution:** The daemon processes one `SessionCommand` at a time.
- **Blocked Heartbeats:** When `exec_command` is waiting for terminal output, the main loop in `run_daemon` is not receiving from the `heartbeat_rx` channel.
- **Peer Timeout:** If the PTY blocks (as described in §2) and the command hangs for > 60 seconds, the remote peer will drop the connection because it has not received a heartbeat from the daemon.

## 4. OS-Level Limits (`ARG_MAX`)

While `ARG_MAX` (typically 256KB to 2MB) limits the size of arguments passed to a *new* process, it does not apply to data "typed" into an existing shell via a PTY. Thus, `ARG_MAX` is not the limiting factor for `rustdesk-cli exec`.

## 5. Recommendations and Best Practices

### A. Use File Transfer for Payloads
For deploying scripts, model weights, or any data > 1KB, **always use the File Transfer protocol**.
- It uses 128KB blocks.
- It bypasses PTY and shell line-length limits.
- It is significantly more robust and supports resuming.

### B. Split Large Commands
If you must use `exec` for data slightly over 4KB, split the data into multiple smaller lines (e.g., using `base64 -w 100` to insert newlines every 100 characters) to avoid hitting the PTY's `MAX_INPUT` limit.

### C. Refactor Daemon Heartbeats
The daemon's heartbeat logic should be moved to a task that is not blocked by command execution, or the `exec_command` loop must explicitly handle heartbeats to keep the session alive during long-running tasks.
