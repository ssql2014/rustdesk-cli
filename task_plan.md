# Task Plan: Implement Terminal Session Support

## Goal
Add direct terminal-session support using the existing RustDesk terminal protobufs, including a terminal-mode connection flow, `OpenTerminal` after login, stdin/stdout streaming, and CLI wiring in `src/main.rs`.

## Current Phase
Phase 5

## Phases
### Phase 1: Requirements & Discovery
- [x] Read skill instructions and existing planning files
- [x] Read terminal protocol research notes
- [x] Inspect `proto/message.proto`, `src/connection.rs`, `src/terminal.rs`, and `src/main.rs`
- **Status:** complete

### Phase 2: Connection Flow Refactor
- [x] Reuse the existing relay/login flow with configurable `ConnType`
- [x] Support terminal login union in `LoginRequest`
- [x] Keep the desktop/default path unchanged
- **Status:** complete

### Phase 3: Terminal Session Implementation
- [x] Add direct terminal connect/open helpers
- [x] Stream local stdin to `TerminalAction::Data`
- [x] Stream `TerminalResponse::Data` to local stdout
- [x] Close cleanly on EOF / remote close
- **Status:** complete

### Phase 4: CLI Wiring
- [x] Expose terminal mode in `src/main.rs`
- [x] Reuse the existing connection arguments and password handling
- [x] Keep existing daemon-based commands intact
- **Status:** complete

### Phase 5: Verification
- [x] Run `cargo build`
- [x] Resolve any compile errors or warnings introduced by the change
- [x] Summarize behavior and remaining risks
- **Status:** complete

## Key Questions
1. Which parts of the current desktop connection flow should be generalized rather than duplicated?
2. Which exact protobuf fields must be set for terminal mode (`ConnType`, `LoginRequest.union`, `TerminalAction`)?
3. What is the least disruptive CLI shape for exposing direct terminal mode?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Reuse the existing `src/terminal.rs` message helpers instead of replacing the file | The repo already contains well-tested terminal action/response helpers |
| Add a reusable connection-mode path instead of copy-pasting `src/connection.rs` | Terminal mode only changes a few protocol fields; duplication would drift |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
|       | 1       |            |

## Notes
- The research docs explicitly require `ConnType::TERMINAL`, `LoginRequest` terminal union, and `OpenTerminal` after login.
- `src/terminal.rs` already has protocol helpers and tests for `TerminalAction` / `TerminalResponse`.
