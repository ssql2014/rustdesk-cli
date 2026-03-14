# Findings & Decisions

## Research Findings
- `docs/research/key_exchange_client_response.md` and the upstream RustDesk client agree on the current format:
  - server `KeyExchange.keys[0]` is a signed payload carrying the server ephemeral X25519 public key
  - the client reply is `[client_ephemeral_x25519_pk, crypto_box(session_key, zero_nonce, server_ephemeral_pk, client_ephemeral_sk)]`
  - the post-handshake stream uses the existing secretbox-style framed transport
- The older `docs/research/tcp_key_exchange.md` note is stale on two points:
  - it describes a sealed-box reply
  - it describes the server payload as a permanent Ed25519 key rather than a signed ephemeral key

## Primary Source Correction
- The upstream `secure_tcp()` path in `rustdesk-official/src/common.rs` verifies `ex.keys[0]` as a signed message and then extracts a 32-byte key from the verified payload.
- The upstream client response is produced by `create_symmetric_key_msg()`, which uses zero-nonce `crypto_box`, not `crypto_box_seal`.
- The local `src/crypto.rs` helper already matches that client-response format.

## Existing Code Findings
- `complete_tcp_key_exchange()` was already using the correct local `crypto::key_exchange_curve25519()` helper.
- The high-risk assumption was the extraction in `verify_rendezvous_server_key()`, which previously hard-coded `signature || key` when falling back without verification.
- That fallback is risky for self-hosted deployments because signature verification may be intentionally skipped when `--key` does not match the hbbs signing key.

## Crypto / Transport Findings
- `Cargo.toml` already has the required crypto primitives; no new dependency was needed.
- `src/transport.rs` and `src/crypto.rs` already match the upstream framing and encrypted-stream behavior.
- The decryption-failure suspicion narrowed to payload extraction rather than the client response construction.

## Live Verification Findings
- The requested live command had to be translated to the current CLI form:
  - `cargo run -- connect 308235080 --terminal ...`
- On the tested server, the TCP punch path did **not** emit `KeyExchange`.
- Instead, hbbs reset the TCP punch socket, the client fell back to `RequestRelay`, connected to hbbr, and successfully reached the terminal prompt after `SignedId`.
- That means the exact live `KeyExchange.keys[0]` bytes from this server were not observable in this run, so the parsing hardening is defensive rather than a reproduced wire-format fix.

## Changes Applied
- Added debug logging in `complete_tcp_key_exchange()` for:
  - `keys[0]` length and first 16 bytes
  - extracted `peer_box_pk`
  - response `sealed_key` length
- Refactored rendezvous key extraction to:
  - accept raw 32-byte payloads
  - verify and accept `signature || key`
  - verify and accept `key || signature`
  - keep a permissive fallback for self-hosted mismatch cases, with explicit warnings
- Added focused tests for both signed payload layouts.

## Verification
- `cargo build` passed.
- Live command equivalent succeeded:
  - hbbs TCP punch reset
  - relay fallback succeeded
  - terminal prompt opened
- `cargo test` passed fully.

## Follow-Up Findings
- The provided `--key` can legitimately differ from the hbbs signing key on self-hosted servers.
- When signature verification is skipped, choosing the wrong 32-byte slice from a 96-byte payload would produce exactly the decryption failure the bug report describes.
- Supporting both verified layouts is therefore a practical compatibility fix even though the upstream client favors `signature || key`.

## Visual/Browser Findings
- None.
