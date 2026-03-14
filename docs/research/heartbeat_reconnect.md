# Heartbeat and Auto-Reconnect Protocol Research

This document details how the RustDesk protocol maintains active connections and recovers from network failures.

## 1. Heartbeat Mechanisms

RustDesk employs different heartbeat strategies depending on the transport layer and connection purpose.

### UDP Heartbeats (Rendezvous)
The client maintains its presence on the rendezvous server (`hbbs`) by repeatedly sending registration messages.
- **Message:** `RegisterPeer` (UDP) or `RegisterPk` (TCP).
- **Interval:** 15 seconds (`REG_INTERVAL`).
- **Purpose:** Keeps the NAT mapping alive in routers and ensures the server knows the client's current public IP/port.

### TCP / Session Heartbeats
Once a session (Desktop, Terminal, or File Transfer) is established, the client and peer exchange heartbeats to monitor link health.
- **Message:** **Empty Payload**. The client sends a `BytesCodec` header with a length of 0.
- **Behavior:** On receipt of empty bytes, the receiver typically responds with its own empty byte heartbeat.
- **WebSocket:** For WSS connections, this also triggers standard WebSocket ping/pong frames.

### Dedicated HealthCheck (HC)
The client establishes a separate TCP connection to `hbbs` specifically for status monitoring.
- **Message:** `HealthCheck` (Protobuf variant 26).
- **Purpose:** Allows the server to track the client's online status independently of active peer-to-peer sessions.

## 2. Timeout and Drop Detection

- **Default Timeout:** 60 seconds (`DEFAULT_KEEP_ALIVE`).
- **Detection Logic:** The client tracks `last_recv_msg` (Instant).
- **Threshold:** A connection is considered dropped if no message (including heartbeats) is received within **90 seconds** (`keep_alive * 1.5`).
- **Implementation:** 
  ```rust
  if last_recv_msg.elapsed().as_millis() as u64 > rz.keep_alive as u64 * 3 / 2 {
      bail!("Connection timeout");
  }
  ```

## 3. Reconnection Strategy

### The Mediator Loop
The official client uses a high-level supervisor loop in `rendezvous_mediator.rs`.
1. **Attempt:** Calls `Self::start(server, host)`.
2. **Failure:** If `start` returns an error (socket reset, timeout, DNS fail), the error is logged.
3. **Backoff:** The loop performs a `sleep` (typically several seconds) before the next attempt.
4. **Retry:** The entire connection sequence (Rendezvous -> Handshake -> Login) is restarted from scratch.

### Terminal Session Persistence
For terminal sessions, reconnection is seamless if persistence was requested:
- The client saves the `service_id` received in the first `TerminalOpened` response.
- During reconnection, the client includes this `service_id` in the `LoginRequest` (field 16).
- The peer then re-attaches the connection to the existing PTY instead of spawning a new shell.

## 4. Implementation Requirements for rustdesk-cli

To improve daemon robustness:
1. **Background Heartbeat:** Add a `tokio::spawn` task to the daemon that sends empty byte heartbeats every 15-30 seconds.
2. **Watchdog Loop:** Wrap the `connection::connect` logic in a loop that detects `Recv` errors and automatically re-initiates the handshake.
3. **Persist service_id:** If `rustdesk-cli exec` is used, the daemon must store the `service_id` to allow subsequent `exec` commands to reuse the same remote environment.
