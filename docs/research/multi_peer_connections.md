# Research: Concurrent Multi-Peer Connections

This document investigates the feasibility and design for supporting multiple simultaneous peer connections within a single `rustdesk-cli` daemon.

## 1. Current Daemon Architecture Analysis

The current `rustdesk-cli` daemon is designed for a single-session lifecycle.

- **Storage:** The `run_daemon` loop maintains a single `EncryptedStream` and a single `Session` object.
- **State:** The `peer_id` is passed as a command-line argument to the daemon and remains fixed for its entire lifetime.
- **Limitation:** To connect to a second peer, the current daemon must be disconnected and restarted with a new `peer_id`, or a second daemon process must be spawned (which is currently blocked by the global `/tmp/rustdesk-cli.sock` path).

## 2. Supporting Distributed Inference (Tensor Parallelism)

Running LLM inference across multiple machines (e.g., splitting a 70B model across two 24GB GPUs on different peers) requires the CLI to orchestrate data flow between multiple active streams.

- **Requirement:** The daemon must hold $N$ active connections.
- **Workflow:** 
    1. `connect peer_A` (Daemon adds peer_A to its map).
    2. `connect peer_B` (Daemon adds peer_B to its map).
    3. `exec --peer peer_A --command "matmul block_1"`
    4. `exec --peer peer_B --command "matmul block_2"`
    5. Local CLI merges results.

## 3. UDS Protocol Enhancements

To support multi-peer routing, the newline-delimited JSON protocol over the Unix Domain Socket must be updated.

### Current Request:
```json
{"Exec": {"command": "ls", "timeout": 30}}
```

### Proposed Multi-Peer Request:
```json
{
  "peer_id": "308235080",
  "command": {"Exec": {"command": "ls", "timeout": 30}}
}
```
If `peer_id` is omitted, the daemon should default to the "primary" (first connected) peer or return an error if multiple are active.

## 4. Official Client Multi-Session Handling

The official RustDesk client handles multiple sessions via a tabbed interface in the **Connection Manager**.
- **Process Model:** It typically uses a single main process that manages multiple `Connection` tasks.
- **Task Isolation:** Each connection runs in its own `tokio::spawn` task with its own `EncryptedStream` and message handling loop.
- **Event Bus:** An internal event bus or channel system routes UI actions (like keystrokes) to the active tab's connection task.

## 5. Proposed Design for rustdesk-cli

### Daemon-Side Changes:
1. **Session Map:** Replace the single `encrypted` and `session` variables with a `HashMap<String, ActiveSession>`.
   ```rust
   struct ActiveSession {
       stream: EncryptedStream<TcpTransport>,
       state: Session,
       service_id: Option<String>, // for terminal resume
   }
   ```
2. **Command Routing:** The UDS listener task should parse the target `peer_id` and dispatch the command to the corresponding entry in the `HashMap`.
3. **Background Tasks:** Each peer in the map must have its own background heartbeat task to prevent timeouts.

### CLI-Side Changes:
1. **Peer Selection:** Add a global `--peer` (or `-p`) flag to all subcommands.
2. **Persistence:** The `connect` command should be updated to allow adding peers to an existing daemon instead of bailing with "Daemon already running".

## Conclusion

Transitioning to multi-peer support is a significant but necessary step for distributed AI inference. By moving to a `HashMap`-based session manager and updating the UDS IPC to include routing metadata, we can support complex multi-node orchestration while maintaining the efficient daemon-client architecture.
