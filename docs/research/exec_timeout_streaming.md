# Research: Exec Timeout and Streaming Implementation

This document details the research findings for implementing robust timeouts and real-time streaming for command execution in `rustdesk-cli`.

## 1. Current Timeout Implementation

The command execution logic in `src/daemon.rs:exec_command` manages three distinct timeout phases:

| Phase | Duration | Description |
| :--- | :--- | :--- |
| **Terminal Open** | 15s | Time allowed for the peer to respond to `OpenTerminal`. |
| **Prompt Drain** | 2s | Idle timeout used to flush the initial shell banner/prompt. |
| **Completion** | **30s (Default)** | Total time allowed for the command to produce the sentinel marker. |

### Code Trace:
- **Default Value:** `DEFAULT_EXEC_COMPLETION_TIMEOUT` in `src/daemon.rs`.
- **Enforcement:** The `exec_command` loop calculates a `deadline` (Instant) and uses `tokio::time::timeout` for each `recv_terminal_data` call, with the duration clipped to the remaining time until the deadline.
- **Limitation:** The current loop is **blocking**; it buffers all received bytes into a `Vec<u8>` and does not return control until the sentinel marker is found or the timeout expires.

## 2. Official Client Streaming Analysis

The official RustDesk client (`rustdesk/rustdesk`) differentiates between interactive and non-interactive (API-like) usage:

### Interactive Streaming
In `src/client/io_loop.rs`, incoming `TerminalResponse::Data` messages are immediately dispatched to the UI or standard output. Each message contains a `bytes` payload representing a partial chunk of the remote PTY output.

### Protocol Support for Partial Output
The `TerminalResponse` protobuf message (variant index 2) wraps a `TerminalData` message:
```protobuf
message TerminalData {
  int32 terminal_id = 1;
  bytes data = 2;
  bool compressed = 3;
}
```
The server sends these as soon as data arrives in the remote PTY's read buffer. There is no protocol-level requirement to wait for a command to finish before sending data.

## 3. Proposed Design for rustdesk-cli

To support long-running inference tasks (>30s), we must move from a **request-response** model to a **streaming-event** model.

### A. Custom Deadlines
- **CLI Flag:** Add `--timeout <secs>` to the `exec` command.
- **Validation:** Clamp the timeout between 1s and 3600s (1 hour).
- **Enforcement:** Pass this value through the UDS IPC to the daemon to override `DEFAULT_EXEC_COMPLETION_TIMEOUT`.

### B. Real-time Output Streaming
Modify the UDS IPC protocol between the CLI and the Daemon to support a multi-message response flow.

**Sequence:**
1.  **Request:** CLI $\rightarrow$ Daemon: `Exec { command, stream: true, ... }`.
2.  **Ack:** Daemon $\rightarrow$ CLI: `SessionResponse::ok("Streaming started")`.
3.  **Data Events:** As the Daemon receives `TerminalData` chunks from the peer, it writes them to the UDS socket as newline-delimited JSON objects:
    `{"success":true, "data": {"stream": "stdout", "chunk": "..."}}`
4.  **Final Response:** Once the sentinel is detected, the Daemon sends the final exit code:
    `{"success":true, "data": {"exit_code": 0, "kind": "complete"}}`

## 4. Summary of Protocol Support

| Feature | Protobuf Message | Implementation Status |
| :--- | :--- | :--- |
| **Real-time Output** | `TerminalResponse::Data` | Supported by peer; needs streaming logic in daemon. |
| **Partial Delivery** | `TerminalData.data` | Supported; no size limit beyond framing. |
| **Error Handling** | `TerminalResponse::Error` | Supported; immediately aborts execution. |
| **Exit Code** | (None) | **Custom Sentinel Required.** The protocol does not natively signal process exit codes for terminal sessions. |

## Conclusion

To unblock the inference pipeline, we must prioritize **UDS streaming**. This will allow the agent to monitor token generation in real-time rather than waiting for the entire 30s+ process to finish. The existing sentinel-based exit code detection remains necessary as the RustDesk protocol itself only provides a raw byte stream for terminal sessions.
