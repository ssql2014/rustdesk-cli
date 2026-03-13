# Findings & Decisions

## Requirements
- Write the design to `/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN.md`.
- Cover exact command syntax, flags, and output format for connection management, screen capture, input control, and batch mode.
- Specify `--json` behavior, exit codes, and screenshot metadata conventions.
- Include side-by-side equivalent commands for `vncdotool`.
- Keep the design practical and minimal, optimized for AI agents.

## Research Findings
- The target project directory is empty, so the deliverable can define the interface from first principles.
- The requested baseline subcommands already constrain the design strongly; the main open design space is output conventions, batch semantics, and ergonomics for agents.
- `vncdotool` uses a small verb set such as `type`, `key`, `move`, `click`, `drag`, and `capture`, and supports chaining many actions in one invocation.
- `vncdotool` click behavior is button-oriented, while the requested `rustdesk-cli` design is coordinate-oriented for clicks; that is a useful divergence for agents because it makes a click self-contained.
- In `vncdotool`, a click commonly follows a separate `move`; the RustDesk CLI should keep click target coordinates inline so each action is self-contained.
- `vncdotool` exposes region capture as a distinct verb, but the RustDesk CLI can stay smaller by handling that with `capture --region`.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Use terse, stable stdout messages in text mode | Agents can still inspect text output when `--json` is not used |
| Reserve stderr for errors and diagnostics | Keeps stdout machine-friendly |
| Treat `do` as a sequential command queue executed by one process | Matches `vncdotool`-style batching while preserving one connection context |
| Make `click` require coordinates | Prevents hidden pointer-state dependencies that exist in some VNC tools |
| Print screenshot metadata to stdout after successful capture | Gives agents a cheap verification signal without opening the image |
| Fold region capture into `capture --region` | Smaller API surface than separate capture verbs |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| No existing project docs or code to infer conventions from | Design from the user’s requested command list and agent-centric constraints |

## Resources
- `/Users/qlss/Documents/Projects/rustdesk-cli`
- `/Users/qlss/.codex/skills/planning-with-files/SKILL.md`
- `https://pypi.org/project/vncdotool/`
- `https://vncdotool.readthedocs.io/en/latest/commands.html`

## Visual/Browser Findings
- External documentation confirmed the comparison should mention `move`, `click`, `type`, `key`, `drag`, and `capture` as the closest `vncdotool` equivalents.
