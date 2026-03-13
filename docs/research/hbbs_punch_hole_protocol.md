# Research Findings: hbbs Punch-Hole and Relay Protocol

This document outlines the behavior of the RustDesk rendezvous server (`hbbs`) regarding `PunchHoleRequest` and `RequestRelay` messages, specifically focusing on the use of `licence_key` and `force_relay`.

## 1. PunchHoleRequest Handling

The handling of `PunchHoleRequest` is located in `src/rendezvous_server.rs` within the `handle_punch_hole_request` function.

### License Key Validation
- **If `licence_key` is incorrect or empty (and server has a key):** The server returns a `PunchHoleResponse` with `failure: LICENSE_MISMATCH` (enum value `3`). This message is sent back to the **requester** (Client A).
- **If `licence_key` is correct:** The server proceeds to look up the target peer (Client B).

### Success vs. Failure Response Behavior
- **On Success:** If the target peer is found and online, `hbbs` constructs a `PunchHole` message and sends it to the **target peer** (Client B). **No response or confirmation is sent back to the requester (Client A)** via UDP. Client A is expected to wait for Client B to initiate the punch-hole process or for a timeout.
- **On Failure (Offline/Not Exist):** If the peer is offline or does not exist, `hbbs` returns a `PunchHoleResponse` with the appropriate failure code (`OFFLINE=2` or `ID_NOT_EXIST=0`) to the **requester**.

This explains why a client receives a response (failure `3`) when the key is empty, but receives "no response" when the key is correct.

## 2. RequestRelay Handling

### UDP vs. TCP/WebSocket
- **UDP:** `hbbs` does **not** handle `RequestRelay` messages sent via UDP. The `handle_udp` function in `src/rendezvous_server.rs` lacks a match arm for this message type.
- **TCP/WebSocket:** `RequestRelay` is explicitly handled in the `handle_tcp` function. When received over TCP, `hbbs` stores the requester's TCP "sink," encodes the requester's IP into the message, and forwards the `RequestRelay` to the target peer via UDP.

### Role of hbbr
`hbbr` is the dedicated relay server. While `hbbs` brokers the *request* for a relay, the actual data connection is established by both clients connecting to `hbbr`.

## 3. Client Flow for `force_relay`

### The `force_relay` Field
- Although `force_relay` exists as a boolean field (index `8`) in the `PunchHoleRequest` protobuf definition, it is **completely ignored** by the current `hbbs` implementation.
- The server only forces a relay based on its own internal logic:
    1. If the environment variable `ALWAYS_USE_RELAY=Y` is set.
    2. If it detects a cross-network connection (one peer is on a LAN and the other is on a WAN).
- In these cases, it overwrites the `nat_type` to `SYMMETRIC` in the `PunchHole` message sent to Peer B, which effectively forces a relay fallback in the RustDesk client.

### Correct Implementation Strategy
If a client wants to guarantee a relay connection:
1. It should connect to `hbbs` via **TCP** (typically port 21116).
2. Send a `RequestRelay` message.
3. Wait for a `RelayResponse` containing the `relay_server` address and a token/UUID.
4. Connect to the `hbbr` server provided in the response.

## Summary Table

| Feature | Behavior in hbbs (UDP) | Behavior in hbbs (TCP) |
| :--- | :--- | :--- |
| **PunchHoleRequest (Success)** | Forwarded to Peer B; **No response** to A | Forwarded to Peer B; No response to A |
| **PunchHoleRequest (Fail)** | Response sent to A (Failure code) | Response sent to A (Failure code) |
| **RequestRelay** | **Ignored** | Handled and forwarded to Peer B |
| **force_relay field** | **Ignored** | **Ignored** |
| **licence_key check** | Performed at start of request | Performed at start of request |
