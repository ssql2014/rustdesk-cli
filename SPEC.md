# rustdesk-cli — Product Spec

**Owner:** Ada (PM, Team Evas)
**Status:** Draft v2 (updated per DESIGN.md)
**Date:** 2026-03-13

## Problem

AI agents need to control remote machines programmatically. RustDesk provides excellent remote desktop infrastructure (NAT traversal, encryption, cross-platform), but its client requires a GUI. There is no CLI equivalent — unlike VNC which has `vncdotool`.

## Goal

A headless command-line RustDesk client that lets AI agents connect to and control remote machines via the RustDesk protocol. Think `vncdotool` but for RustDesk.

## Non-Goals

- Replacing the RustDesk GUI client
- File transfer (future scope)
- Audio/clipboard sync
- Acting as a RustDesk server/relay

## Core Commands (MVP)

```
rustdesk-cli connect <id> [--password <pw>] [--server <addr>]
rustdesk-cli disconnect
rustdesk-cli capture [<file.png>] [--format png|jpg] [--quality <0-100>]
rustdesk-cli type "<text>"
rustdesk-cli key <keyname> [--modifiers ctrl,shift,alt,meta]
rustdesk-cli click <x> <y> [--button left|right|middle] [--double]
rustdesk-cli move <x> <y>
rustdesk-cli drag <x1> <y1> <x2> <y2>
rustdesk-cli scroll <x> <y> <delta>
rustdesk-cli status                        # connection state, screen resolution
rustdesk-cli do <step...>                  # batch mode: chain commands in one invocation
```

### Usage Patterns

**Single command (stateless):**
```bash
rustdesk-cli connect 123456789 --password secret
rustdesk-cli capture screen.png
rustdesk-cli click 500 300
rustdesk-cli type "hello world"
rustdesk-cli key enter
rustdesk-cli disconnect
```

**Piped sequence (like vncdotool):**
```bash
rustdesk-cli connect 123456789 --password secret \
  && rustdesk-cli capture before.png \
  && rustdesk-cli click 500 300 \
  && rustdesk-cli type "search query" \
  && rustdesk-cli key enter \
  && sleep 2 \
  && rustdesk-cli capture after.png \
  && rustdesk-cli disconnect
```

**Session persistence:** After `connect`, the session is maintained via a local daemon/socket so subsequent commands reuse the connection without re-authenticating.

## Key Requirements

| # | Requirement | Priority |
|---|------------|----------|
| 1 | Written in Rust | Must |
| 2 | No GUI dependencies (no Flutter, no Sciter, no X11) | Must |
| 3 | RustDesk ID + password authentication | Must |
| 4 | NAT traversal via RustDesk rendezvous/relay servers | Must |
| 5 | Screenshot capture: decode single video frame → PNG | Must |
| 6 | Keyboard input: text and special keys | Must |
| 7 | Mouse: click, move, drag, scroll | Must |
| 8 | Session persistence via Unix domain socket | Must |
| 9 | Capture latency < 500ms (frame decode + PNG encode) | Should |
| 10 | Custom rendezvous server support | Should |
| 11 | stdout output for capture (pipe to other tools) | Should |
| 12 | JSON output mode (`--json`) for programmatic use | Should |
| 13 | Connection timeout and retry options | Should |
| 14 | Works on Linux and macOS (headless) | Must |

## Architecture

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ rustdesk-cli │────▶│ rustdesk-cli     │────▶│ RustDesk Server │
│ (commands)   │ UDS │ daemon           │ TCP │ (rendezvous +   │
│              │◀────│ (session holder) │◀────│  relay)          │
└─────────────┘     └──────────────────┘     └─────────────────┘
                           │                         │
                           │    RustDesk Protocol     │
                           │    (encrypted P2P/relay) │
                           ▼                         ▼
                    ┌─────────────────┐     ┌──────────────┐
                    │ Remote machine  │◀───▶│ RustDesk     │
                    │                 │     │ Server (host)│
                    └─────────────────┘     └──────────────┘
```

### Components

1. **CLI frontend** — Parses commands, communicates with daemon via Unix domain socket (`/tmp/rustdesk-cli.sock`). Thin layer, no network logic.

2. **Session daemon** — Spawned by `connect`, holds the RustDesk connection alive. Receives commands over UDS, translates to RustDesk protocol messages. Exits on `disconnect` or timeout.

3. **Protocol layer** — Reuses RustDesk crates:
   - `hbb_common` — protobuf messages, crypto, networking primitives
   - `rendezvous_mediator` — NAT hole-punching, rendezvous server communication
   - Video decoding: use `vpx` (VP8/VP9) or `aom` (AV1) to decode single frames — no continuous rendering

4. **Capture pipeline** — Request a single video frame → decode → encode as PNG → write to file or stdout. No frame buffer, no continuous stream.

### Crate Reuse from rustdesk/rustdesk

| Crate | What we use |
|-------|------------|
| `hbb_common` | Protobuf definitions, `tcp::FramedStream`, encryption, config |
| `rendezvous_mediator` | Peer connection setup, NAT traversal |
| `libs/hbb_common/protos` | Message types: `LoginRequest`, `MouseEvent`, `KeyEvent`, `VideoFrame` |

### What We Don't Use

- `flutter/` — entire Flutter GUI
- `sciter/` — legacy GUI
- `libs/scrap` — screen capture (we're the *client*, not the host)
- Any platform windowing (winit, gtk, cocoa)

## Session Lifecycle

1. **Connect:** CLI spawns daemon → daemon contacts rendezvous server → NAT hole-punch or relay → authenticate with password → session established → daemon writes PID + socket path to `/tmp/rustdesk-cli.lock`

2. **Commands:** CLI reads lock file → connects to UDS → sends command → daemon executes over RustDesk protocol → returns result via UDS → CLI prints to stdout

3. **Disconnect:** CLI sends disconnect → daemon closes RustDesk connection → removes socket + lock file → exits

4. **Timeout:** Daemon self-terminates after configurable idle period (default: 5 min)

## Error Handling

All errors go to stderr. Exit codes:
- `0` — success
- `1` — connection error (unreachable, auth failed)
- `2` — session error (no active session, disconnected)
- `3` — input error (bad arguments, invalid coordinates)

## Security Considerations

- Passwords via `--password` flag, `RUSTDESK_PASSWORD` env var, or stdin (`--password-stdin`)
- Daemon socket is user-only permissions (0600)
- No credentials written to disk
- All traffic encrypted end-to-end (RustDesk protocol)

## Open Questions

1. **RustDesk crate extraction** — How cleanly can we extract `hbb_common` and connection logic from the monorepo? May need to vendor or fork specific modules.
2. **Video codec** — RustDesk uses VP9 by default. Need to confirm we can decode a single keyframe without a full streaming decoder context.
3. **Protocol stability** — RustDesk protocol isn't versioned or documented as stable. We may need to pin to a specific RustDesk version.
4. **Relay fallback** — When direct P2P fails, relay is needed. Need to understand relay server requirements and rate limits.

## Team & Responsibilities

| Role | Person | Scope |
|------|--------|-------|
| PM / Spec | Ada | This spec, priorities, coordination |
| Researcher | Nova | RustDesk protocol analysis, crate extraction feasibility |
| Designer | Leo | CLI API/UX, error messages, output formats |
| Dev / QA | Max | Implementation, testing, CI |

## Milestones

| Phase | Deliverable | Criteria |
|-------|------------|----------|
| **M0: Spike** | Can we establish a RustDesk connection from headless Rust code? | Successful auth + receive 1 video frame |
| **M1: Connect + Capture** | `connect`, `disconnect`, `capture` working | Screenshot of remote machine saved as PNG |
| **M2: Input** | `type`, `key`, `click`, `move` working | Can type text and click buttons remotely |
| **M3: Polish** | Session daemon, error handling, JSON output, docs | Reliable enough for AI agent loops |
| **M4: Release** | Published crate + binary | `cargo install rustdesk-cli` works |

## References

- [RustDesk source](https://github.com/rustdesk/rustdesk)
- [vncdotool](https://github.com/sibson/vncdotool) — inspiration for CLI design
- [RustDesk protocol (protobuf)](https://github.com/rustdesk/rustdesk/tree/master/libs/hbb_common/protos)
