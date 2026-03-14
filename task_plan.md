# Task Plan: Debug HBBS TCP KeyExchange Decryption Failure

## Goal
Confirm the exact hbbs TCP `KeyExchange.keys[0]` wire format, instrument the local handshake path, make parsing tolerant if necessary, and verify with build, a live terminal connection attempt, and the test suite.

## Current Phase
Phase 4

## Phases
### Phase 1: Discovery
- [x] Read the `planning-with-files` skill instructions
- [x] Read `docs/research/key_exchange_client_response.md`
- [x] Inspect current TCP rendezvous flow in `src/rendezvous.rs`
- [x] Inspect official RustDesk client / server source for the KeyExchange format
- **Status:** complete

### Phase 2: Design
- [x] Confirm whether `keys[0]` is still `signature || ephemeral_x25519_pk`
- [x] Decide how to instrument the handshake without destabilizing the transport
- [x] Decide whether to accept multiple possible payload layouts defensively
- **Status:** complete

### Phase 3: Implementation
- [x] Add debug hex logging around `complete_tcp_key_exchange()`
- [x] Make rendezvous key extraction tolerant of both signed payload layouts
- [x] Add focused tests for the accepted layouts
- **Status:** complete

### Phase 4: Verification
- [x] Run `cargo build`
- [x] Run the requested live terminal connection command equivalent for the current CLI
- [x] Run `cargo test`
- [x] Summarize what the live server actually did
- **Status:** complete

## Key Questions
1. Does the live hbbs server actually send a TCP `KeyExchange` on this path?
2. Is the server payload `signature || key`, `key || signature`, or something else?
3. If verification fails, what is the safest fallback extraction rule for self-hosted deployments?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Follow `key_exchange_client_response.md` and the upstream `secure_tcp` client flow, not the older simplified TCP note | The newer primary-source research matches the local crypto helpers |
| Add handshake logging before changing core crypto primitives | The failure needed live wire evidence more than another crypto rewrite |
| Accept both `signature || key` and `key || signature` layouts when a verifiable 32-byte key can be recovered | This is a low-risk compatibility guard for self-hosted / version-skewed servers |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| The current CLI no longer has the `direct` subcommand from the bug report | 1 | Used the equivalent `connect <peer> --terminal ...` invocation |
| The live hbbs server did not emit `KeyExchange`; it reset the TCP punch socket and the client fell back to relay | 1 | Verified the terminal session still completed and kept the instrumentation/tests for when a keyed hbbs path is exercised |

## Notes
- The live server used in verification did not hit the TCP `KeyExchange` path on the tested terminal flow.
- `proto/rendezvous.proto` already defines `KeyExchange`, so the remaining issue is runtime behavior rather than schema.

## Follow-Up Fixes
- Issue `#39`: when the provided key does not verify the hbbs TCP signature, the client now logs a warning and proceeds with the embedded rendezvous key instead of aborting the handshake.
- Issue `#40`: `PUNCH_HOLE_RESPONSE_TIMEOUT` was increased from `2s` to `6s` to account for the longer encrypted TCP rendezvous flow.
