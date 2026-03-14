# Research Findings: RustDesk Client Relay Flow

This document details how the official RustDesk client (from the `rustdesk/rustdesk` repository) handles relay requests.

## 1. Transport Mechanism

The official client uses **TCP** to send `RequestRelay` messages to the Rendezvous Server (`hbbs`).

- **Target Port:** Port 21116 (default `RENDEZVOUS_PORT`).
- **Function:** `request_relay` in `src/client.rs`.
- **Implementation:** It calls `connect_tcp(rendezvous_server, CONNECT_TIMEOUT)` to establish a new TCP connection for each attempt.

## 2. Connection Lifecycle

1.  **Handshake:** If a `key` and `token` are provided, the client performs `secure_tcp` over the established TCP connection before sending the relay request. This is primarily to protect the authentication token.
2.  **Request:** A `RendezvousMessage` containing the `RequestRelay` union variant is sent. This message includes:
    - `id`: Target peer ID.
    - `uuid`: A newly generated version 4 UUID.
    - `token`: Authentication token.
    - `relay_server`: Preferred relay server address.
    - `secure`: Boolean flag for encryption.
3.  **Response:** The client waits for a `RelayResponse` from `hbbs` over the same TCP connection.
4.  **Relay Connection:** Once a successful `RelayResponse` is received, the client closes the TCP connection to `hbbs` and initiates a new TCP connection to the specified **Relay Server** (port 21117) via `create_relay`.
5.  **Meeting:** In `create_relay`, the client sends another `RequestRelay` message to the `hbbr` server to identify itself and "meet" the target peer.

## 3. Retry Logic

The `request_relay` function implements a fixed retry loop:
- **Attempts:** Up to **3 attempts** (`for i in 1..=3`).
- **Rationale:** The code comment notes: *"use different socket due to current hbbs implementation requiring different nat address for each attempt"*. Since a new TCP connection is created for each iteration, a different source port is used.
- **Timeout:** Each `connect_tcp` and message read operation is wrapped in a timeout (default `CONNECT_TIMEOUT` or `READ_TIMEOUT`).

## 4. Why UDP is not used for RequestRelay

Based on the research:
- `hbbs` (server) does not have a handler for `RequestRelay` in its UDP loop.
- The official client explicitly uses `connect_tcp` in the `request_relay` function.
- Using TCP ensures reliable delivery of the request and response, and allows for the `secure_tcp` layer to protect sensitive tokens.

## Conclusion for rustdesk-cli

To maintain compatibility with official `hbbs` servers, `rustdesk-cli` **must** use TCP when requesting a relay. The current UDP-only approach for all rendezvous messages will fail for relay requests.
