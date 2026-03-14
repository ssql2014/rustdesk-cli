# Task Plan: Fix HBBS TCP KeyExchange in Rendezvous Flow

## Goal
Implement the hbbs TCP `KeyExchange` handshake described in `docs/research/tcp_key_exchange.md` so `punch_hole_via_tcp_with_conn_type()` can upgrade to an encrypted TCP rendezvous stream before sending `PunchHoleRequest`, then verify the build and relevant tests.

## Current Phase
Phase 4

## Phases
### Phase 1: Discovery
- [x] Read the `planning-with-files` skill instructions
- [x] Read `docs/research/tcp_key_exchange.md`
- [x] Inspect current TCP rendezvous flow in `src/rendezvous.rs`
- [x] Inspect available crypto and transport helpers
- **Status:** complete

### Phase 2: Design
- [x] Determine whether existing dependencies can perform sealed-box crypto
- [x] Determine whether existing encrypted transport can be reused
- [x] Define the new hbbs TCP handshake helper shape
- **Status:** complete

### Phase 3: Implementation
- [x] Add TCP KeyExchange handshake logic
- [x] Send encrypted `PunchHoleRequest` after handshake
- [x] Add or update focused tests for KeyExchange handling
- **Status:** complete

### Phase 4: Verification
- [x] Run `cargo build`
- [x] Run targeted rendezvous tests
- [x] Summarize behavior and residual risks
- **Status:** complete

## Key Questions
1. Can `crypto_box`’s sealed-box support cover the server hello / client response without adding a new dependency?
2. Does hbbs TCP use the existing `EncryptedStream` framing, or does it need a separate ChaCha20-Poly1305 transport wrapper?
3. Should the handshake remain internal to `punch_hole_via_tcp_with_conn_type()` or return a new transport abstraction?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Reuse existing `crypto_box` crate for sealed-box encryption if possible | Avoid adding sodiumoxide unless the current dependency set is insufficient |
| Follow the official RustDesk `secure_tcp` flow over the simplified research summary | The upstream client is the primary source for the real wire behavior |
| Reuse the existing `EncryptedStream` after the hbbs TCP handshake | Official source uses a secretbox-style encrypted framed stream compatible with the existing local transport model |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| The research summary described a simpler one-key/ChaCha handshake than the upstream client actually uses | 1 | Checked the official RustDesk source and implemented the signed-ephemeral-key + encrypted replay flow instead |
| `src/rendezvous.rs` is compiled directly inside some integration tests with local `#[path = ...]` modules | 1 | Updated `tests/live_server_test.rs` to include the matching local `transport` and `crypto` modules |

## Notes
- `PunchHoleRequest` over TCP currently goes out in plaintext and only accepts plaintext `PunchHoleResponse` / `RelayResponse`.
- `proto/rendezvous.proto` already defines `KeyExchange`.
