# Task Plan: Fix Background Daemon Handoff

## Goal
Make `cargo run -- connect <peer>` keep the background daemon alive after the parent exits, so a follow-up `cargo run -- status` still reports `connected`.

## Current Phase
Phase 3

## Phases
### Phase 1: Inspect Spawn / Handoff
- [x] Read `src/main.rs` connect path
- [x] Read `src/daemon.rs` spawn and startup path
- [x] Identify where readiness is signaled too early
- **Status:** complete

### Phase 2: Patch Daemon Startup
- [x] Fully detach the spawned daemon from the parent process group
- [x] Delay lock-file readiness until the connection stream is fully initialized
- [x] Preserve startup errors if initialization fails after auth
- **Status:** complete

### Phase 3: Verify
- [x] Run live `connect` against peer `308235080`
- [x] Wait 3 seconds and run `status`
- [x] Run `cargo test`
- **Status:** complete

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Detach the child with `setsid()` in `spawn_daemon()` | The foreground `--daemon` path was stable, while the spawned background process was not; the missing process-group/session detach was the most plausible handoff gap |
| Move `LockFile::write()` until after `initialize_stream_for_mode()` | The parent should not report `connected` until the post-auth stream is actually usable |
| Keep verification against the live peer plus the full test suite | This bug only showed up in the real background handoff path |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| `cargo test` initially failed in CLI “no active session” tests | 1 | The live handoff test left a daemon session active during the test run; reran `cargo test` in a clean state and it passed |

