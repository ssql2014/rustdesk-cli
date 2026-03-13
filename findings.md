# Findings & Decisions

## Requirements
- Add CLI integration tests in `/Users/qlss/Documents/Projects/rustdesk-cli/tests/cli_test.rs`.
- Add `assert_cmd` and `predicates` to `[dev-dependencies]`.
- Test `--help`, JSON output for each command, `do --json`, exit codes, and valid/invalid `--region`.
- Run `cargo test` and keep the current stubbed command behavior.

## Research Findings
- There is currently no `tests/` directory, so the integration suite will be added from scratch.
- The binary already emits stable JSON payloads, so tests can parse stdout as JSON and assert individual fields.
- For invalid `--region`, clap should fail argument parsing before command execution, so the test should assert failure and error text rather than runtime exit code `0`.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Use terse, stable stdout messages in text mode | Agents can still inspect text output when `--json` is not used |
| Reserve stderr for errors and diagnostics | Keeps stdout machine-friendly |
| Treat `do` as a sequential command queue executed by one process | Matches `vncdotool`-style batching while preserving one connection context |
| Make `click` require coordinates | Prevents hidden pointer-state dependencies that exist in some VNC tools |
| Print screenshot metadata to stdout after successful capture | Gives agents a cheap verification signal without opening the image |
| Fold region capture into `capture --region` | Smaller API surface than separate capture verbs |
| Use one output helper path for text and JSON | Reduces drift between command modes |
| Stub `status` as disconnected and other commands as successful placeholders | Avoids inventing fake persistent state while preserving the designed output contract |
| Use helper functions in tests to parse stdout into `serde_json::Value` | Keeps repeated assertions compact and readable |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| No existing project docs or code to infer conventions from | Design from the user’s requested command list and agent-centric constraints |
| The scaffold has no shared output model | Add structured payload builders and final render helpers in `main.rs` |
| No existing CLI tests | Add an integration suite against the built binary with `assert_cmd` |

## Resources
- `/Users/qlss/Documents/Projects/rustdesk-cli`
- `/Users/qlss/.codex/skills/planning-with-files/SKILL.md`
- `https://pypi.org/project/vncdotool/`
- `https://vncdotool.readthedocs.io/en/latest/commands.html`

## Visual/Browser Findings
- External documentation confirmed the comparison should mention `move`, `click`, `type`, `key`, `drag`, and `capture` as the closest `vncdotool` equivalents.
