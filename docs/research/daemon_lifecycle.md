# Research: Daemon Lifecycle and Process Management

This document details how the official RustDesk client manages background processes, PID files, and signal handling, providing a roadmap for hardening the `rustdesk-cli` daemon.

## 1. Process Architecture

The official client employs a split architecture:
- **UI Process (Client/FFI):** The user-facing application (Flutter/Sciter).
- **Server Process:** The background task that handles incoming and outgoing connections, video encoding, and PTY management.

## 2. PID File and Stale Detection

RustDesk uses a robust detection pattern located in `src/ipc.rs`:

1.  **Socket Path:** Located via `Config::ipc_path()`. On Unix, this is typically `/tmp/rustdesk.sock`.
2.  **PID File:** Created as `{socket_path}.pid`.
3.  **Startup Sequence (`check_pid`):**
    - Read the PID from the `.pid` file.
    - Use the `sysinfo` crate to verify if a process with that PID is currently running.
    - **Name Verification:** Check if the running process's name matches "rustdesk".
    - **Socket Probe:** Attempt to `connect()` to the existing socket. If connection succeeds, the daemon is truly alive; if it fails with `ECONNREFUSED`, the file is stale.
4.  **Stale Cleanup:** If the PID check fails or the socket connection is refused, the client calls `std::fs::remove_file()` on the socket path before attempting a new `bind()`.

## 3. Signal Handling and Shutdown

The official client ensures a clean exit through the following mechanisms:

- **Signal Trapping:** Uses `ctrlc::set_handler` to catch `SIGINT` and `SIGTERM`.
- **Global Cleanup:** A `common::global_clean()` function is called at the end of `main()`.
- **Service Draining:** Background services (Audio, Video, Terminal) are notified to exit by dropping their input channels. For example, in `terminal_service.rs`, dropping the `input_tx` signals the writer thread to terminate.
- **IPC Closure:** The IPC listener is stopped, and the socket/pid files are unlinked (if shutdown was graceful).

## 4. Multi-Session Management

The background service is natively multi-session:
- **`ConnMap`:** A `HashMap<i32, ConnInner>` stores all active peer connections.
- **Task Isolation:** Each connection is handled by a separate `tokio::spawn` task.
- **Service IDs:** For terminal sessions, a `HashMap` of `PersistentTerminalService` is maintained to allow multiple independent PTYs or reconnections to the same PTY.

## 5. Implementation Recommendations for rustdesk-cli

To align with official patterns and fix known bugs:

1.  **Fix Socket Bind Bug:** Implement the `check_pid` and "Connect-then-Unlink" pattern. If `bind()` fails with `EADDRINUSE`, attempt to connect to the socket. If it fails, delete the file and retry `bind()`.
2.  **Add PID File:** Store the daemon's PID in `/tmp/rustdesk-cli.sock.pid` to enable the verification check described in §2.
3.  **Graceful Exit:** 
    - Implement a `Shutdown` signal (via `tokio::sync::broadcast`) that all background tasks (heartbeat, UDS listener, Peer receiver) subscribe to.
    - On `SIGTERM`, broadcast the signal and wait for all tasks to join before deleting the socket file.
4.  **Multi-Peer Preparation:** Transition the current single `Session` state to a `HashMap<String, Session>` to support multiple simultaneous connections.
