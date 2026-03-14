# Findings & Decisions

## Requirements
- Read `docs/research/terminal_proto_additions.md` and `docs/research/post_login_protocol.md`.
- Reuse the existing connection flow from `src/connection.rs` for a terminal-specific connect path.
- Use `ConnType::TERMINAL` in the rendezvous/relay flow.
- Send `LoginRequest` with the terminal union set.
- After `LoginResponse`, send `TerminalAction(OpenTerminal)`.
- Stream local stdin to `TerminalAction::Data` and terminal output back to stdout.
- Wire the new mode into `src/main.rs`.
- Verify with `cargo build`.

## Research Findings
- `proto/rendezvous.proto` defines `ConnType::TERMINAL = 5`.
- `proto/message.proto` defines:
  - `LoginRequest.union.terminal`
  - `Message.union.terminal_action`
  - `Message.union.terminal_response`
  - `OpenTerminal { terminal_id, rows, cols }`
  - `TerminalData { terminal_id, data, compressed }`
- `src/terminal.rs` already contains tested helpers for:
  - sending `TerminalAction`
  - waiting for `TerminalOpened`
  - sending `TerminalData`
  - decoding `TerminalResponse::Data`, `Closed`, and `Error`
- `src/connection.rs` currently hard-codes desktop/default mode in three places:
  - UDP `PunchHoleRequest.conn_type`
  - TCP `RequestRelay.conn_type`
  - `LoginRequest.union` is always `None`
- `src/main.rs` currently exposes an existing daemon-backed `shell` command, but there is no direct terminal-mode connection path.
- The implemented shape is `rustdesk-cli connect <peer-id> --terminal ...` rather than replacing the existing daemon-backed `shell` command.
- Terminal mode now reuses the shared connection path with:
  - `ConnType::Terminal` in PunchHole, RequestRelay-over-TCP, and relay binding
  - `LoginRequest.union = Terminal { service_id: "" }`
  - `open_terminal()` immediately after `LoginResponse`
  - a byte-streaming stdin/stdout loop in `src/terminal.rs`

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Generalize the connection flow with a configurable mode | Needed to keep desktop and terminal login behavior aligned |
| Reuse `src/terminal.rs` for protocol framing and add session orchestration there | Keeps terminal-specific behavior in one module |
| Expose terminal mode on the connect path rather than replacing daemon shell support | Avoids breaking the current `shell` workflow while adding the requested direct terminal session |
| Reject `--json` with `connect --terminal` | Interactive stdout streaming and JSON output are incompatible on the same stdout channel |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| The repo already has `src/terminal.rs`, despite the request saying to create it | Reused and extended the existing file instead of duplicating terminal logic |

## Resources
- `docs/research/terminal_proto_additions.md`
- `docs/research/post_login_protocol.md`
- `proto/message.proto`
- `proto/rendezvous.proto`
- `src/connection.rs`
- `src/terminal.rs`
- `src/main.rs`

## Visual/Browser Findings
- None.
