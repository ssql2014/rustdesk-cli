# Task: Text-Mode Optimization Architecture (Leo — Architect)

AFTER you finish DESIGN_TEXT_SESSION.md, pick this up as your next task.

## Context
Our primary use case is terminal/text work over RustDesk for AI agents.
We need to optimize for maximum efficiency in text mode.

## Assess and design the following optimizations:

### P0 — Must Have:
1. **Minimal keystroke latency** — fastest path from local input to remote PTY
2. **Smart buffering** — batch rapid keystrokes, stream output efficiently
3. **Terminal resize (SIGWINCH)** — proper tmux/vim usage on remote
4. **Raw PTY mode** — clean pass-through of escape sequences (colors, cursor, etc.)
5. **Compression for terminal output** — TerminalData has a `compressed` flag, design how to use it

### P1 — Should Have:
6. **Clipboard sync** — copy/paste between local and remote via Clipboard protobuf messages
7. **Command execution mode** — send command, get stdout, no interactive PTY (for `exec`)
8. **Multiplexed channels** — multiple terminal sessions over one connection (terminal_id field supports this)

### P2 — Nice to Have (assess feasibility):
9. **Type-ahead** — buffer local keystrokes during high latency
10. **Local echo** — for low-latency feel
11. **Delta updates** — only send changed terminal lines (like mosh)

### Skip:
- Video decoding / screenshot rendering (deprioritize)
- Mouse event handling (terminal users use keyboard)
- High-FPS frame delivery

## Output:
Add a section to DESIGN_TEXT_SESSION.md (or create DESIGN_TEXT_OPTIMIZATIONS.md) covering:
- Which optimizations are feasible given RustDesk's protocol
- Recommended implementation order
- Any protocol limitations or workarounds needed
- Module structure for the optimization layer
