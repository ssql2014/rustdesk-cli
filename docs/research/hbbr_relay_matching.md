# Relay Matching and Peer Connection Research

This document explains how the RustDesk relay server (`hbbr`) and the peer-side connection logic handle session pairing and connection types.

## 1. hbbr Matching Logic

Analysis of `src/relay_server.rs` in the `rustdesk-server` repository confirms:
- **Key:** Pairing is based **exclusively on the UUID string**.
- **ConnType:** The `conn_type` field in the `RequestRelay` message is **ignored** by `hbbr`. It is not used for matching or verification.
- **Protocol:** `hbbr` decodes the initial `RequestRelay` from both sides, matches the UUIDs, and then enters a transparent "raw" relay mode where it pipes bytes between the two sockets.

## 2. Peer-Side Behavior (Controlled Side)

Analysis of `src/server.rs` and `src/rendezvous_mediator.rs` in the official client repo reveals:
- **Default ConnType:** The peer **always** sends `ConnType::DefaultConn (0)` when connecting to `hbbr`. It does not mirror the connection type requested by the client at the relay level.
- **UUID Generation:** When handling a `PunchHole` message with `force_relay: true`, the peer generates its own **UUID v4** and expects the client to connect to it.
- **Dual Requests:** If the client sends both a `PunchHoleRequest` and a `RequestRelay` (with different UUIDs) more than 100ms apart, the peer will initiate **two separate** relay connections.

## 3. The Root Cause of the Hang

The hang in `direct --terminal` mode likely stems from the **UUID Mismatch/Race**:

1.  Client sends `PunchHoleRequest` (UUID_A).
2.  Peer receives `PunchHole`, generates **UUID_Peer**, connects to `hbbr`, and sends `RelayResponse(UUID_Peer)` to the client.
3.  Client (busy sleeping/ignoring) sends `RequestRelay` (UUID_Client) to `hbbs`.
4.  Peer receives `RequestRelay`, connects to `hbbr(UUID_Client)`.
5.  Client connects to `hbbr(UUID_Client)`.
6.  **The Conflict:** `hbbr` pairs the client with the peer's *second* connection attempt. However, the peer's *first* attempt (UUID_Peer) might still be active in the background, or the peer's internal session manager might be confused by receiving two relay requests for the same Peer ID in a short window.

## 4. Correct Protocol Flow

To match the official client and avoid hangs:

1.  **Phase 2:** Send `PunchHoleRequest` and **wait** for a response.
2.  **Response Handling:**
    - If `PunchHoleResponse` is received: Use the peer's IP/port (direct) or the `relay_server` (relay).
    - If `RelayResponse` is received (common for `force_relay`): **Immediately use the UUID and relay address provided by the peer.** Do not send a separate `RequestRelay`.
3.  **Relay Binding:** When connecting to `hbbr`, use `ConnType::DefaultConn` to match the peer's handshake behavior.
4.  **Login:** Only specify `ConnType::Terminal` inside the encrypted `LoginRequest` (Step 7).

## Conclusion

The `rustdesk-cli` implementation must be more reactive to the rendezvous server's responses. Forcing our own UUID via a second TCP request while the peer is already trying to establish a relay via the first UDP request is the most likely cause of the protocol hang.
