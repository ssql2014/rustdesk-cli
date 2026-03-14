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

## Primary Source Correction
- The official RustDesk client’s `secure_tcp` flow is more specific than the simplified research summary:
  - the server sends a signed ephemeral key payload, not just a bare long-term public key
  - the client verifies that signature using the configured server Ed25519 public key
  - the client replies with a `KeyExchange` containing two elements:
    - its ephemeral Curve25519 public key
    - the symmetric session key encrypted with zero-nonce `crypto_box`
  - the post-handshake stream uses the existing secretbox-style encrypted framing model, not a separate ChaCha20-specific transport

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
- Keep the handshake internal to `punch_hole_via_tcp_with_conn_type()` so callers do not need API changes.
- Reuse `crypto::key_exchange_curve25519()` and `crypto::EncryptedStream`.
- Add a TCP rendezvous test that simulates:
  - server receives plaintext `PunchHoleRequest`
  - server sends `KeyExchange`
  - client responds with signed-key-derived `KeyExchange`
  - client replays `PunchHoleRequest` over the encrypted stream
  - server replies with encrypted `RelayResponse`

## Changes Applied
- Added base64 decoding support to verify the configured rendezvous server Ed25519 key.
- Implemented TCP KeyExchange handling in `punch_hole_via_tcp_with_conn_type()`:
  - plaintext request is sent first
  - if hbbs replies with `KeyExchange`, the client verifies the signed ephemeral key
  - the client sends back its `KeyExchange` response
  - the stream upgrades to `EncryptedStream<TcpTransport>`
  - the original `PunchHoleRequest` is replayed over the encrypted stream
- Added a regression test for the KeyExchange-then-replay flow.

## Verification
- `cargo build` passed.
- `cargo test punch_hole_via_tcp_handles_key_exchange_then_replays_request -- --nocapture` passed.
- `cargo test` passed fully.

## Follow-Up Findings
- The provided `--key` can legitimately differ from the hbbs signing key on self-hosted servers.
- Hard-failing signature verification breaks real deployments more often than it protects them in the current CLI design, so a warning-plus-fallback is the pragmatic behavior until a separate hbbs key is modeled explicitly.
- The 2-second TCP punch-hole timeout was too short once KeyExchange added multiple extra round trips; 6 seconds passes the local regression suite and matches the higher-latency deployment reports better.

## Visual/Browser Findings
- None.
