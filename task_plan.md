# Task Plan: rustdesk-cli CLI Design, Implementation, and Testing

## Goal
Lock down the `rustdesk-cli` CLI contract with integration tests that verify help output, JSON responses, batch mode, exit codes, and `--region` parsing against the current stubbed implementation.

## Current Phase
Phase 5

## Phases
### Phase 1: Requirements & Discovery
- [x] Understand user intent
- [x] Inspect the current `Cargo.toml`
- [x] Confirm there is no existing `tests/` directory
- **Status:** complete

### Phase 2: Planning & Structure
- [x] Define test coverage areas
- [x] Choose `assert_cmd` and `predicates` test approach
- [x] Plan JSON parsing assertions
- **Status:** complete

### Phase 3: Implementation
- [x] Add dev-dependencies
- [x] Create `tests/cli_test.rs`
- [x] Cover help, JSON outputs, batch mode, exit codes, and region parsing
- **Status:** complete

### Phase 4: Testing & Verification
- [x] Run `cargo test`
- [x] Fix test failures if any
- [x] Confirm all requested assertions are present
- **Status:** complete

### Phase 5: Delivery
- [x] Review changed files
- [x] Ensure deliverables are complete
- [ ] Deliver summary to user
- **Status:** in_progress

## Key Questions
1. Which response fields should be asserted exactly versus only for presence?
2. How should region parse failures be tested when clap exits before command execution?
3. How much of the batch JSON payload should be locked down in this first test pass?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Prefer a small subcommand set over many aliases | Keeps the agent interface predictable and easier to script |
| Make `--json` a global machine-output mode | Gives agents one consistent parsing contract across commands |
| Keep text output one line per command success | Simple for humans, still stable for logs and fallback parsing |
| Add per-step results in `do --json` output | Agents need to know exactly which step failed without replaying logs |
| Keep command implementations stubbed but typed | Lets the CLI surface stabilize before transport/session logic exists |
| Parse JSON in tests instead of string-matching whole objects | Keeps tests stable across harmless field-order changes |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| Missing `PredicateBooleanExt` import in tests | 1 | Added the trait import and reran `cargo test` |

## Notes
- Re-read this plan before major decisions.
- Keep `DESIGN.md` as the source of truth for output format and flags.

## 2026-03-14 Unit Test Expansion

### Goal
Add focused unit tests for session state transitions and protocol helpers, then verify they pass alongside the existing CLI integration suite.

### Phases
#### Phase 1: Discovery
- [x] Read `src/session.rs`
- [x] Read `src/protocol.rs`
- [x] Confirm current integration test coverage in `tests/cli_test.rs`
- **Status:** complete

#### Phase 2: Implementation
- [x] Add `#[cfg(test)]` coverage in `src/session.rs`
- [x] Add `#[cfg(test)]` coverage in `src/protocol.rs`
- **Status:** complete

#### Phase 3: Verification
- [x] Run `cargo test`
- [x] Fix any compile/test failures
- [x] Confirm both unit and integration suites pass
- **Status:** complete

## 2026-03-14 Drag And Scroll Session Commands

### Goal
Add session-layer drag and scroll commands that emit the expected mouse protocol messages, then verify they pass with the existing unit and integration coverage.

### Phases
#### Phase 1: Discovery
- [x] Read `src/session.rs`
- [x] Read `src/protocol.rs`
- [x] Read `DESIGN.md`
- [x] Confirm where `SessionCommand` is used
- **Status:** complete

#### Phase 2: Implementation
- [x] Add `Drag` and `Scroll` to `SessionCommand`
- [x] Add scroll mask constants to `MouseEvent`
- [x] Implement drag and scroll dispatch behavior
- [x] Add unit tests for the new commands
- **Status:** complete

#### Phase 3: Verification
- [x] Run `cargo test`
- [x] Confirm unit and integration suites still pass
- **Status:** complete

## 2026-03-14 Transport Layer

### Goal
Add a reusable transport abstraction with RustDesk-style length-prefixed framing, wire it into the crate, and verify it with unit and integration tests.

### Phases
#### Phase 1: Discovery
- [x] Read `TASK_LEO.md`
- [x] Read `src/protocol.rs`
- [x] Read `src/daemon.rs`
- [x] Check current crate/module state before editing
- **Status:** complete

#### Phase 2: Implementation
- [x] Create `src/transport.rs`
- [x] Add the `Transport` trait
- [x] Implement `TcpTransport`
- [x] Implement `FramedTransport`
- [x] Add duplex-based framing tests
- [x] Add `mod transport;` to `src/main.rs`
- **Status:** complete

#### Phase 3: Verification
- [x] Run `cargo test`
- [x] Fix compile or test regressions
- [x] Confirm full suite passes cleanly
- **Status:** complete

## 2026-03-14 Rendezvous Client

### Goal
Add a UDP rendezvous client built on the prost-generated RustDesk signaling types, cover registration/hole-punch/relay flows with unit tests, and keep the full crate test suite green.

### Phases
#### Phase 1: Discovery
- [x] Read `RESEARCH.md` sections 8 and 10
- [x] Inspect `src/proto.rs`
- [x] Inspect the generated `hbb.rs` types
- [x] Confirm current crate/module state before editing
- **Status:** complete

#### Phase 2: Implementation
- [x] Create `src/rendezvous.rs`
- [x] Implement `RendezvousClient::connect`
- [x] Implement `register_peer`
- [x] Implement `punch_hole`
- [x] Implement `request_relay`
- [x] Add UDP unit tests
- [x] Add `mod rendezvous;` to `src/main.rs`
- **Status:** complete

#### Phase 3: Verification
- [x] Run `cargo test`
- [x] Confirm unit and integration suites pass
- **Status:** complete
