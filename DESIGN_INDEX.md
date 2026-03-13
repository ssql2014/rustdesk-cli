# DESIGN_INDEX

Brief navigation aid for the current architecture docs.

## [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md)

- Scope:
  - daemon-owned text session architecture
  - real connect flow
  - terminal lifecycle
  - exec model
  - clipboard protocol usage
  - runtime state, routing, reconnection, and module graph
- Covers:
  - Issue #20 foundation
  - text-first daemon wiring
  - connect -> shell/exec -> disconnect lifecycle
- Cross-references:
  - base document for all other text-mode design docs
  - extended by [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md)
  - shell transport is concretized in [`DESIGN_SHELL_STREAMING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_SHELL_STREAMING.md)
  - local IPC is concretized in [`DESIGN_UDS_PROTOCOL.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_UDS_PROTOCOL.md)

## [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md)

- Scope:
  - P0/P1/P2 text-mode optimizations
  - latency, buffering, resize, raw PTY passthrough, compression
  - clipboard sync, exec specialization, multiplexed terminal channels
  - type-ahead/local echo/delta-update feasibility
- Covers:
  - Issue #20
  - optimization feasibility and implementation order
- Cross-references:
  - assumes the daemon/session model from [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md)
  - shell-specific usage is applied in [`DESIGN_SHELL_STREAMING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_SHELL_STREAMING.md)

## [`DESIGN_ERROR_HANDLING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_ERROR_HANDLING.md)

- Scope:
  - stderr vs stdout routing
  - exit code mapping
  - batch validation model
  - disconnect no-session semantics
  - plain-text and JSON error formatting
- Covers:
  - Issue #24
  - `BUG-013`
  - `BUG-009`
  - `BUG-012`
  - `BUG-014`
- Cross-references:
  - should be applied to all CLI/daemon command paths, including shell attach and UDS protocol error frames
  - aligns with request/response error envelopes in [`DESIGN_UDS_PROTOCOL.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_UDS_PROTOCOL.md)

## [`DESIGN_SHELL_STREAMING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_SHELL_STREAMING.md)

- Scope:
  - interactive shell bidirectional streaming
  - local raw terminal mode
  - signal handling
  - shell session cleanup
  - latency path analysis
  - shell-specific failure recovery
- Covers:
  - Issue #26
  - interactive `shell` behavior over daemon UDS
- Cross-references:
  - builds directly on [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md)
  - integrates buffering/latency/compression choices from [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md)
  - wire format is specified in [`DESIGN_UDS_PROTOCOL.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_UDS_PROTOCOL.md)

## [`DESIGN_UDS_PROTOCOL.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_UDS_PROTOCOL.md)

- Scope:
  - exact CLI<->daemon Unix socket wire protocol
  - frame format
  - version negotiation
  - control request/response messages
  - shell stream frames
  - backpressure rules
- Covers:
  - Issue #28
  - Max’s implementation contract for #18
- Cross-references:
  - concretizes the shell upgrade model from [`DESIGN_SHELL_STREAMING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_SHELL_STREAMING.md)
  - should carry structured errors consistent with [`DESIGN_ERROR_HANDLING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_ERROR_HANDLING.md)
  - assumes daemon/session ownership model from [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md)

## Suggested Reading Order

1. [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md)
2. [`DESIGN_UDS_PROTOCOL.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_UDS_PROTOCOL.md)
3. [`DESIGN_SHELL_STREAMING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_SHELL_STREAMING.md)
4. [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md)
5. [`DESIGN_ERROR_HANDLING.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_ERROR_HANDLING.md)
