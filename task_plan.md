# Task Plan: rustdesk-cli CLI Design

## Goal
Write a practical `DESIGN.md` that specifies the CLI API and user experience for `rustdesk-cli`, covering command syntax, flags, output conventions, batch mode, and a side-by-side comparison with `vncdotool`.

## Current Phase
Phase 5

## Phases
### Phase 1: Requirements & Discovery
- [x] Understand user intent
- [x] Identify constraints and requirements
- [x] Document findings in findings.md
- **Status:** complete

### Phase 2: Planning & Structure
- [x] Define technical approach
- [x] Create document structure
- [x] Document decisions with rationale
- **Status:** complete

### Phase 3: Implementation
- [x] Draft the CLI API design
- [x] Write `DESIGN.md`
- [x] Keep the design practical and minimal
- **Status:** complete

### Phase 4: Testing & Verification
- [x] Verify all requested commands are covered
- [x] Verify exit codes and JSON behavior are specified
- [x] Check comparison with `vncdotool`
- **Status:** complete

### Phase 5: Delivery
- [x] Review written files
- [x] Ensure deliverables are complete
- [ ] Deliver summary to user
- **Status:** in_progress

## Key Questions
1. What CLI surface is minimal but still useful for AI agents?
2. How should human-readable output and `--json` output coexist cleanly?
3. How should batch mode behave around sequencing, failures, and observability?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Prefer a small subcommand set over many aliases | Keeps the agent interface predictable and easier to script |
| Make `--json` a global machine-output mode | Gives agents one consistent parsing contract across commands |
| Keep text output one line per command success | Simple for humans, still stable for logs and fallback parsing |
| Add per-step results in `do --json` output | Agents need to know exactly which step failed without replaying logs |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| None | 1 | No errors so far |

## Notes
- Re-read this plan before major decisions.
- Keep the document focused on agent workflows instead of human-admin tooling.
