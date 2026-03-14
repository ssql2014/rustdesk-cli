# Task Plan: Fix HBBS TCP KeyExchange in Rendezvous Flow

## Goal
Implement the hbbs TCP `KeyExchange` handshake described in `docs/research/tcp_key_exchange.md` so `punch_hole_via_tcp_with_conn_type()` can upgrade to an encrypted TCP rendezvous stream before sending `PunchHoleRequest`, then verify the build and relevant tests.

## Current Phase
Phase 2

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
- [ ] Define the new hbbs TCP handshake helper shape
- **Status:** in_progress

### Phase 3: Implementation
- [ ] Add TCP KeyExchange handshake logic
- [ ] Send encrypted `PunchHoleRequest` after handshake
- [ ] Add or update focused tests for KeyExchange handling
- **Status:** pending

### Phase 4: Verification
- [ ] Run `cargo build`
- [ ] Run targeted rendezvous tests
- [ ] Summarize behavior and residual risks
- **Status:** pending

## Key Questions
1. Can `crypto_box`’s sealed-box support cover the server hello / client response without adding a new dependency?
2. Does hbbs TCP use the existing `EncryptedStream` framing, or does it need a separate ChaCha20-Poly1305 transport wrapper?
3. Should the handshake remain internal to `punch_hole_via_tcp_with_conn_type()` or return a new transport abstraction?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Reuse existing `crypto_box` crate for sealed-box encryption if possible | Avoid adding sodiumoxide unless the current dependency set is insufficient |
| Treat the current `EncryptedStream` as likely non-reusable until proven otherwise | Research says hbbs TCP uses ChaCha20-Poly1305, while the existing type is XSalsa20-Poly1305 |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
|       | 1       |            |

## Notes
- `PunchHoleRequest` over TCP currently goes out in plaintext and only accepts plaintext `PunchHoleResponse` / `RelayResponse`.
- `proto/rendezvous.proto` already defines `KeyExchange`.
