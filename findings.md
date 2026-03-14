# Findings & Decisions

## Research Findings
- `hbbs` may send a TCP `RendezvousMessage::KeyExchange` before accepting functional requests.
- The required handshake is:
  1. connect raw TCP
  2. receive `KeyExchange` with server Ed25519 public key
  3. generate random 32-byte symmetric key
  4. sealed-box that key to the server public key
  5. send `KeyExchange` response with the sealed box
  6. switch to an encrypted stream
  7. send `PunchHoleRequest` over that encrypted stream

## Existing Code Findings
- `src/rendezvous.rs` `punch_hole_via_tcp_with_conn_type()` currently:
  - opens raw TCP
  - sends plaintext `PunchHoleRequest`
  - reads framed `RendezvousMessage`s
  - only accepts `PunchHoleResponse`, `RelayResponse`, and `RegisterPeerResponse`
- `src/rendezvous.rs` `request_relay_via_tcp_with_conn_type()` also assumes plaintext `RelayResponse`.
- `src/connection.rs` expects `punch_hole_via_tcp_with_conn_type()` to hide the transport details and return only logical rendezvous results.

## Crypto / Transport Findings
- `Cargo.toml` already depends on `crypto_box = 0.9`, and the installed crate exposes `PublicKey::seal`, which implements `crypto_box_seal`.
- The repo already has `TcpTransport::new(TcpStream)` and BytesCodec framing, so the same framing can be preserved after handshake.
- The existing `crate::crypto::EncryptedStream` is not a direct fit:
  - it uses `XSalsa20-Poly1305`
  - the new research explicitly calls for `ChaCha20-Poly1305`

## Likely Implementation Direction
- Add a small encrypted transport wrapper local to rendezvous TCP, or add a generic ChaCha20-Poly1305 encrypted framed transport helper.
- Keep the handshake internal to `punch_hole_via_tcp_with_conn_type()` so callers do not need API changes.
- Add a TCP rendezvous test that simulates:
  - server sends `KeyExchange`
  - client responds with sealed key
  - subsequent encrypted request is a `PunchHoleRequest`
  - server replies with encrypted `RelayResponse` or `PunchHoleResponse`

## Visual/Browser Findings
- None.
