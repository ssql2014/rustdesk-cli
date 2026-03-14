# Research: Connection Pooling and Session Multiplexing

This document explores how the RustDesk protocol and official client handle session reuse and identifies opportunities for optimization in `rustdesk-cli`.

## 1. File Transfer Session Reuse

Analysis of `src/server/connection.rs` reveals that file transfer sessions are designed to be long-lived and handle multiple jobs.

- **Tracking:** The `Connection` struct uses a `HashSet<i32>` named `cm_read_job_ids` to track active file reading tasks.
- **Multiplexing:** Multiple `FileAction` requests (e.g., listing a directory while a large file is uploading) can be sent over the same TCP connection. The peer uses the `id` field in the Protobuf message to route responses to the correct job.
- **Cleanup:** When a job completes (`Done`) or fails (`Error`), the `id` is removed from the tracking set, but the TCP connection remains open for subsequent actions.

## 2. Multiplexing Constraints

The protocol enforces strict separation between session types.

- **Initialization Lock:** The `ConnType` (Desktop, Terminal, File Transfer) is defined at login.
- **State Dependency:** Server-side handlers depend on state initialized during the `LoginRequest`. For example, `handle_terminal_action` requires a `terminal_user_token` which is only generated if the `LoginRequest` was a `Terminal` variant.
- **Conclusion:** You **cannot** send terminal commands over a file transfer session or vice versa. Each requires its own dedicated TCP stream.

## 3. Connection Setup Overhead

Establishing a new secure session is computationally and network-intensive.

| Phase | Steps | Network Cost |
| :--- | :--- | :--- |
| **Discovery** | DNS + Rendezvous | 2-3 RTTs |
| **Transport** | TCP Handshake | 1 RTT |
| **Security** | NaCl Key Exchange | 2-3 RTTs |
| **Auth** | Login Request/Response | 1 RTT |
| **TOTAL** | | **~6-8 RTTs** |

On a high-latency link (e.g., 150ms), a new connection takes **over 1 second** just for the handshake. For an AI agent running 20 sequential commands, this adds **20 seconds** of dead time.

## 4. Proposed Design for rustdesk-cli

To eliminate this overhead, `rustdesk-cli` should implement **Connection Pooling** within the daemon.

### Pool Architecture
- **Storage:** `HashMap<(PeerId, ConnType), ActiveSession>`
- **Behavior:**
    - `exec`: Requests a `TERMINAL` session. If one exists, it sends the command immediately.
    - `push/pull`: Requests a `FILE_TRANSFER` session. Reuses the existing stream for multiple file operations.
- **Maintenance:** The daemon must send heartbeats (empty frames) on all pooled connections to prevent NAT/Relay timeouts (default 60s).

### Implementation Benefits
- **Latency:** Reduces "Cold Start" time for remote commands from ~1.5s to < 100ms.
- **Throughput:** Avoids TCP slow-start on every new command.
- **Reliability:** Centralizes heartbeat and error handling logic.

## Conclusion

The RustDesk protocol supports multiplexing *within* a session type but requires separate connections *between* session types. By caching these specialized connections in a persistent daemon pool, `rustdesk-cli` can provide the near-instant response times required for interactive AI agent workflows.
