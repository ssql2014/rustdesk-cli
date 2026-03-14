# Terminal Session Hang: Deep Dive Research

This document details the findings from researching the official RustDesk client's connection flow to resolve the hang in `direct --terminal` mode.

## 1. Timing and Ordering of Requests

The official client (`src/client.rs`) follows a strict sequential fallback rather than parallel firing:

1.  **Initial Punch:** Sends `PunchHoleRequest` (UDP) to `hbbs`.
2.  **Wait:** It waits for a `PunchHoleResponse` with a timeout (3s, 6s, 9s in a loop).
3.  **Relay Fallback:** Only if `PunchHoleResponse` indicates a relay is needed (or times out) does it establish a TCP connection to `hbbs` to send a `RequestRelay`.
4.  **Avoidance of Conflicts:** By waiting, it ensures the peer isn't receiving multiple conflicting "Relay" instructions with different UUIDs or transport hints simultaneously.

**Recommendation:** `rustdesk-cli` should implement a 200-500ms delay after the UDP punch before attempting the TCP relay fallback.

## 2. Relay Handshake and Binding (`hbbr`)

When a relay connection is established, the following occurs:

1.  **Transport Connect:** Client connects to `hbbr` (TCP 21117).
2.  **Binding Message:** The client **must** send a `RendezvousMessage` containing `RequestRelay` to `hbbr`. 
    - This message must contain the **same UUID** sent to `hbbs`.
    - It must contain the correct **`conn_type`** (e.g., `TERMINAL`).
    - It must contain the `licence_key`.
3.  **Forwarding:** Only after this binding does `hbbr` start forwarding bytes from the peer.
4.  **Handshake Step 1:** The peer's `SignedId` is only received **after** the relay is bound.

**Bug Found:** `rustdesk-cli`'s `relay_connect` implementation was defaulting to `ConnType::DefaultConn` in the `hbbr` binding message, even for terminal sessions.

## 3. ConnType Consistency

`ConnType::TERMINAL` (value 5) must be present in:
- `PunchHoleRequest` (to `hbbs` UDP)
- `RequestRelay` (to `hbbs` TCP)
- `RequestRelay` (to `hbbr` TCP - the binding message)
- `LoginRequest` (the union field)

If the binding message to `hbbr` specifies `DefaultConn`, the peer's server-side logic will initialize a desktop session handler, which sends video frames instead of waiting for a terminal open request.

## 4. UUID Strategy

- **Generation:** Use a fresh UUID v4 for every session attempt.
- **Consistency:** The UUID sent to `hbbs` (the "instruction") must perfectly match the UUID sent to `hbbr` (the "meeting point").
- **Encoding:** The UUID is sent as a string in the Protobuf `uuid` field.

## 5. Conclusion: Why the Hang Happens

The hang at `SignedId` / `Hash` is a result of **Protocol Mismatch**:
1.  The client connects to `hbbr` and binds as `DefaultConn`.
2.  The peer connects to `hbbr` and binds as `Terminal`.
3.  `hbbr` sees the mismatch or the peer's server-side handler rejects the session type change.
4.  The encrypted handshake completes (NaCl is agnostic to `ConnType`), but the peer never sends the `Hash` because it is in an inconsistent state or waiting for desktop-session initialization.

**Fix:** Update `connection.rs` to propagate `ConnType` into the `hbbr` binding message and add a slight delay between UDP and TCP requests.
