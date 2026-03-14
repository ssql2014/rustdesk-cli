# Terminal Session: Direct vs. Daemon Flow Research

This document explains why `rustdesk-cli connect <id> --terminal` (direct mode) often hangs, while `rustdesk-cli exec` (via daemon) works correctly.

## 1. Flow Comparison

| Phase | Direct Terminal Flow (`terminal.rs`) | Daemon Exec Flow (`daemon.rs`) |
| :--- | :--- | :--- |
| **Initial State** | No previous connection. | **Previously connected as `DefaultConn`.** |
| **Registration** | New `my_id` (cli-PID). | Reuses existing `my_id`. |
| **Punch/Relay** | Sends UDP Punch + TCP Relay simultaneously. | Same (during reconnection). |
| **Handshake** | Recv `SignedId`, Send `PublicKey`. | Recv `SignedId`, Send `PublicKey`. |
| **Result** | **Hangs waiting for `Hash`.** | **Recv `Hash`, Send `LoginRequest`, Success.** |

## 2. Root Cause Analysis

### The "Priming" Effect
The daemon flow works because it **primes** the peer. When `run_daemon` starts, it connects as `DefaultConn`. This connection:
1.  Successfully registers the Client ID with `hbbs`.
2.  Sends an `OptionMessage` to the peer.
3.  Establishes a known-good session state on the peer.

When `exec` triggers a reconnection as `Terminal`, the peer already has an active or recently-closed session context for that Client ID.

### Relay Request Conflict
`rustdesk-cli` sends both UDP `PunchHoleRequest` and TCP `RequestRelay` with `force_relay: true`. 
- The peer receives the forwarded `PunchHole` and initiates a relay connection with its own UUID.
- The peer receives the forwarded `RequestRelay` and initiates a relay connection with our UUID.

In `direct --terminal` mode, this race condition seems to prevent the peer from correctly triggering the `on_open` event (which sends the `Hash` salt/challenge). In daemon mode, the previous successful session seems to make the peer's `rendezvous_mediator` more resilient to this conflict.

## 3. Official Client Differences

Research into `rustdesk/rustdesk` source (`src/client.rs`) reveals:
1.  **Wait for Response:** The official client waits for a `PunchHoleResponse` (with a timeout) before deciding whether to request a relay. `rustdesk-cli` fires both and moves on.
2.  **OptionMessage in Login:** The official client includes an `OptionMessage` directly inside the `LoginRequest`. `rustdesk-cli` sends `None` for options during the handshake.
3.  **ConnType Handling:** The server-side `handle_punch_hole` logic in the official repo generates a new UUID if `force_relay` is set, which can conflict if the client also provides a UUID via `RequestRelay`.

## 4. Conclusion and Next Steps

The hang is a protocol-level race condition. 

**Immediate Fix for Leo:**
Update `connection.rs` to wait for a short period (e.g., 200ms) or a `PunchHoleResponse` before sending the TCP `RequestRelay`. This gives the peer time to process the first instruction before being hit by the second.

**Long-term Fix:**
Implement the `OptionMessage` logic within the `LoginRequest` to match the official client's behavior and ensure the peer is correctly configured for terminal mode from the very first byte of the session.
