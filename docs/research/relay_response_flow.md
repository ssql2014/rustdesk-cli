# RelayResponse Flow Research

This document explains the message flow between the client, rendezvous server (`hbbs`), and peer during the establishment of a relay session.

## 1. Message Transport Mismatch

A critical finding in the `hbbs` source code (`src/rendezvous_server.rs`) reveals an asymmetry in how responses are sent back to the requester (Client A):

- **PunchHoleResponse:** Sent via the **same transport** as the request. If the client sent a UDP `PunchHoleRequest`, `hbbs` responds via UDP.
- **RelayResponse:** Forwarded exclusively via **TCP**. When the peer (Client B) connects to the relay and notifies `hbbs`, the server looks up the requester's address in a `tcp_punch` map. If the requester did not establish a TCP connection to `hbbs`, the message is silently dropped.

## 2. Protocol Sequence Comparison

### Official Client Flow (TCP-based Rendezvous)
1. **Client A** opens TCP to `hbbs:21116`.
2. **Client A** sends `PunchHoleRequest` over TCP.
3. **hbbs** forwards `PunchHole` (UDP) to **Client B**.
4. **Client B** connects to `hbbr:21117` and sends `RelayResponse` (TCP) to **hbbs**.
5. **hbbs** forwards `RelayResponse` (TCP) to **Client A**'s open TCP socket.
6. **Client A** receives the peer's UUID and relay info and connects to `hbbr`.

### rustdesk-cli Current Flow (UDP-based Rendezvous)
1. **Client A** sends `PunchHoleRequest` (UDP) to `hbbs:21116`.
2. **Client A** waits for `PunchHoleResponse` (UDP).
3. **hbbs** forwards `PunchHole` (UDP) to **Client B**.
4. **Client B** connects to `hbbr` and sends `RelayResponse` (TCP) to **hbbs**.
5. **hbbs** attempts to forward `RelayResponse` via TCP but fails (no sink).
6. **Client A** times out waiting for UDP response.
7. **Client A** manually sends `RequestRelay` (TCP), creating a race condition on the peer.

## 3. Message Definition

The `RelayResponse` is defined as variant **19** in the `RendezvousMessage` oneof:

```protobuf
message RendezvousMessage {
  oneof union {
    ...
    RelayResponse relay_response = 19;
    ...
  }
}
```

Our project's `proto/rendezvous.proto` is already synchronized with this definition.

## 4. How to Fix the Terminal Hang

To fix the hang in `direct --terminal` mode, the client must be able to receive the peer's generated relay info.

1. **Switch to TCP Rendezvous:** Modify `RendezvousClient` to support TCP connections for `PunchHoleRequest`.
2. **Handle Both Response Types:** The response-waiting loop in `connection.rs` must handle both `PunchHoleResponse` and `RelayResponse`.
3. **UUID Priority:** If a `RelayResponse` is received, the client must use the UUID provided by the peer instead of generating its own. This eliminates the race condition where the peer is told about two different relay UUIDs for the same session.

## Conclusion

The "Offline" hang is primarily a result of the client being "deaf" to the relay coordination messages sent by `hbbs` over TCP. By switching Phase 2 to use TCP, we align with the official client's behavior and ensure reliable relay establishment.
