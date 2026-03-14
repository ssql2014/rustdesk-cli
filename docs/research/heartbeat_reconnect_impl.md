# Heartbeat and Auto-Reconnection Implementation Research

This document details the research and design for robust heartbeats and auto-reconnection in `rustdesk-cli`.

## 1. Current Code Trace

### `src/crypto.rs`
- **`EncryptedStream`**: Maintains `last_recv_at: Instant` updated on every `recv()`.
- **`send_heartbeat()`**: Sends a raw zero-length frame `self.inner.send(&[])`. This bypasses the crypto sequence counters to keep the pipe alive without affecting message ordering.
- **`recv_idle_for()`**: Returns `self.last_recv_at.elapsed()`.

### `src/daemon.rs`
- **Constants**: `HEARTBEAT_INTERVAL` (30s) and `KEEPALIVE_TIMEOUT` (90s).
- **Main Loop**: Uses `tokio::select!` to handle `HeartbeatTick`.
- **Reconnection**: Calls `reconnect_encrypted_stream()` when a heartbeat send fails or the idle timeout is exceeded.

### `src/connection.rs`
- **`handshake_and_auth`**: Performs the NaCl handshake and login. Currently accepts an optional `login_union` which can contain terminal session info.

## 2. Identified Gaps

1. **Incomplete Backoff**: The current `RECONNECT_BACKOFFS` is `[1s, 2s, 4s]`. It stops after 3 attempts, which is insufficient for transient network failures.
2. **State Loss**: If a terminal session is active, the `service_id` is not preserved. Reconnecting spawns a *new* shell instead of resuming the existing one.
3. **Wait for LoginResponse**: The reconnection logic in `daemon.rs` doesn't fully handle the case where the peer is online but the session setup (Step 8) fails due to internal peer state.

## 3. Reconnection State Machine Design

We propose a robust state machine for the daemon:

- **`Connecting`**: Initial connection or starting a fresh retry.
- **`Connected`**: Encrypted stream established, `last_recv_at` being updated.
- **`HeartbeatLost`**: `recv_idle_for() > 90s` OR `send_heartbeat()` error.
- **`Reconnecting`**: Executing the backoff strategy.
- **`Failed`**: Reached max attempts or permanent error (e.g. wrong password).

### Backoff Strategy
Exponential backoff: `1s, 2s, 4s, 8s, 16s, 30s`. Max cap at 30s.

## 4. Terminal Session Persistence (`service_id`)

To support seamless resumption of `rustdesk-cli exec` or `rustdesk-cli shell`:
1. Capture `service_id` from the `TerminalOpened` response message.
2. Store `service_id` in the `Session` struct within `daemon.rs`.
3. During `handshake_and_auth` in a reconnection, pass the `service_id` into the `LoginRequest`:
   ```rust
   login_request::Union::Terminal(Terminal { service_id: stored_id, .. })
   ```

## 5. Proposed Changes to `src/daemon.rs`

1. **Update Constants**:
   ```rust
   const RECONNECT_BACKOFFS: [Duration; 6] = [
       Duration::from_secs(1),
       Duration::from_secs(2),
       Duration::from_secs(4),
       Duration::from_secs(8),
       Duration::from_secs(16),
       Duration::from_secs(30),
   ];
   ```
2. **Track Service ID**: Add `active_service_id: Option<String>` to the daemon loop state.
3. **Retry Loop**: Wrap the peer message handling in a `loop` that catches `should_reconnect` errors and invokes the backoff logic.
4. **Resumable Login**: Update `login_union_for_conn_type` to accept the optional `service_id`.
