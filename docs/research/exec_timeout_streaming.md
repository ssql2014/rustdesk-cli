# Research: Exec Timeout and Streaming

This document details the current implementation of command execution in `rustdesk-cli`, identifies limitations for long-running inference tasks, and proposes a design for custom deadlines and real-time output streaming.

## 1. Current Implementation Analysis

The `rustdesk-cli exec` command is implemented via an ephemeral terminal session in `src/daemon.rs:exec_command`.

### Timeout Handling
- **Default Timeout:** **30 seconds** (defined as `DEFAULT_EXEC_COMPLETION_TIMEOUT`).
- **Enforcement:** The daemon uses a `tokio::time::timeout` wrapper or a manual deadline check within its `loop`.
- **Phases:**
    1. **Terminal Open:** 15s timeout.
    2. **Prompt Drain:** 2s idle timeout (waiting for the remote shell to finish printing its banner).
    3. **Command Execution:** Uses the provided `timeout_secs` or the 30s default.
- **Result on Timeout:** If the deadline is reached before the sentinel marker appears, the daemon returns the partial output collected so far with a `timed_out: true` flag.

### Output Handling
- **Buffering:** Currently, **all output is buffered** in a `Vec<u8>` until the command completes or times out. 
- **Sentinel Mechanism:** Completion is detected by appending `echo '__sentinel__'$?` to the user's command and searching for that unique string in the output stream.

## 2. Protocol and Streaming Support

### Protocol Messages
The RustDesk protocol is naturally streaming-oriented. The relevant messages are:
- **`TerminalAction::Data`**: Client-to-Server stdin.
- **`TerminalResponse::Data`**: Server-to-Client stdout/stderr chunks.

Each `TerminalData` message contains a `bytes` field. The server sends these as soon as data is available from the remote PTY, allowing for sub-second latency in output delivery.

### Real-time vs. Exec
- **Terminal Sessions:** `rustdesk-cli connect --terminal` wires `TerminalResponse::Data` directly to local `stdout`, providing real-time feedback.
- **Exec Commands:** Currently trades real-time feedback for a "clean" request-response model suitable for AI agents. This is the bottleneck for inference observability.

## 3. Proposed Design: Custom Deadlines & Streaming

To support long-running inference runs (e.g., >30s), we need two major enhancements.

### A. Custom Command Deadlines
Add a `--timeout` (or `-t`) flag to the `exec` command.
- **CLI:** `rustdesk-cli exec --command "python infer.py" --timeout 300`
- **UDS IPC:** The `SessionCommand::Exec` variant already supports an `Option<u64>` for timeout. This just needs to be exposed in the CLI frontend.

### B. Real-time Output Streaming
Modify the UDS IPC protocol between the CLI and the Daemon to support a "Stream" mode for `exec`.

**Design Pattern:**
1. **Request:** CLI sends `Exec { command, stream: true, ... }`.
2. **Handshake:** Daemon confirms terminal is open.
3. **Streaming Phase:**
    - As the Daemon receives `TerminalResponse::Data` from the peer, it immediately forwards it over the UDS socket to the CLI process.
    - The CLI process prints these chunks to its `stdout` in real-time.
4. **Completion:** When the sentinel is reached, the Daemon sends a final `SessionResponse` containing the exit code.

## 4. Summary of Protocol Support

| Feature | Message Type | Support Level |
| :--- | :--- | :--- |
| Partial Output | `TerminalResponse::Data` | Native (Streaming) |
| Completion Signal | Custom Sentinel | Implementation-defined |
| Remote Error | `TerminalResponse::Error` | Native |
| Session Exit | `TerminalResponse::Closed` | Native |

## Conclusion

The RustDesk protocol fully supports real-time streaming. The current 30s hang/buffer behavior is an implementation choice in `rustdesk-cli`'s `exec` logic. By exposing the timeout parameter and implementing an IPC streaming mode, we can provide the necessary observability for long-running AI inference tasks.
