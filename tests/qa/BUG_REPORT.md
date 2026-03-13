# QA Bug Report — rustdesk-cli

**Tester:** IVY (QA Agent)
**Date:** 2026-03-14
**Version:** 0.1.0 (commit 710ad33)
**Spec:** SPEC.md (Draft v2)

---

## Critical Bugs

### BUG-001: Connect to invalid/nonexistent server succeeds with exit 0

**Severity:** CRITICAL
**Command:** `rustdesk-cli connect 999999999 --server invalid.example.com --timeout 3`
**Expected:** Exit 1 with connection error (server doesn't exist)
**Actual:** Exit 0, output: `connected id=999999999 server=invalid.example.com width=1920 height=1080`
**Impact:** The CLI reports a successful connection to a nonexistent server with fake resolution data (1920x1080). This means an AI agent using this tool would believe it's connected when it's not. The daemon is spawned, lock/socket files are created, and subsequent commands appear to succeed.
**Notes:** This suggests the `connect` command may be stubbed/mocked and not actually performing a real RustDesk handshake.

### BUG-002: Capture reports success but writes no file

**Severity:** CRITICAL
**Steps:**
1. `rustdesk-cli connect 999999999 --server invalid.example.com --timeout 3` (exits 0)
2. `rustdesk-cli capture test.png`
**Expected:** Either writes a real PNG file, or fails with an error
**Actual:** Exit 0, output `captured file=test.png format=png width=1920 height=1080 bytes=267393` — but `test.png` does not exist on disk.
**Impact:** The CLI lies about having captured a screenshot. An AI agent would try to process a file that doesn't exist.

---

## Major Bugs

### BUG-003: Connect with wrong password succeeds

**Severity:** MAJOR
**Command:** `rustdesk-cli connect 308235080 --password WRONG_PASSWORD --id-server ... --timeout 10`
**Expected:** Exit 1 with authentication error
**Actual:** Exit 0 with `connected` message
**Impact:** No password verification is actually happening. Any password is accepted.
**Notes:** Combined with BUG-001, this confirms the connection/auth flow is not fully implemented.

### BUG-015: `exec` is completely stubbed — never executes remote commands

**Severity:** CRITICAL
**Command:** `rustdesk-cli exec --command "echo hello"` (while connected to live server)
**Expected:** Returns actual output from running `echo hello` on the remote machine
**Actual:** Always returns `output=stub exec output` regardless of what command is passed. Exit code is always 0. Even `exec --command "exit 42"` returns `exit_code=0, output="stub exec output"`.
**JSON output:** `{"command":"exec","exit_code":0,"ok":true,"output":"stub exec output","requested":"echo hello"}`
**Impact:** The `exec` command is non-functional. It pretends to run commands but never actually communicates with the remote machine.

### BUG-016: `exec` JSON format doesn't match PM spec

**Severity:** MEDIUM
**PM says:** exec JSON should have `stdout`, `stderr`, and `exit_code` fields
**Actual:** Has `output` (single string) and `exit_code`. Missing separate `stdout` and `stderr` fields.
**Impact:** Consumers expecting PM-specified format will fail.

### BUG-012: `do` batch command succeeds without active session

**Severity:** MAJOR
**Command:** `rustdesk-cli do type hello key enter` (no active connection)
**Expected:** Exit 1 or 2 with "no active session" error (same as running `type` or `key` individually)
**Actual:** Exit 0, output claims steps executed: `1 typed chars=5 / 2 key key=enter / ok steps=2`
**Impact:** Batch commands bypass the session check. Individual commands correctly fail, but wrapping them in `do` skips the check.

### BUG-013: Error messages go to stdout instead of stderr

**Severity:** MAJOR
**Spec says:** "All errors go to stderr"
**Actual:** Error messages like `connection_error: No active session...` are printed to stdout.
**Test:** `rustdesk-cli type hello 2>/dev/null` still shows the error (proving it's on stdout).
**Impact:** Breaks standard Unix conventions. Scripts that capture stdout will get error messages mixed with data.

---

## Spec Deviations (Medium)

### BUG-004: `scroll` command missing

**Severity:** MEDIUM
**Spec says:** `rustdesk-cli scroll <x> <y> <delta>`
**Actual:** `scroll` is not a recognized subcommand. Running it suggests `shell` instead.
**Impact:** Scroll functionality is entirely missing.

### BUG-005: `click --double` flag missing

**Severity:** MEDIUM
**Spec says:** `rustdesk-cli click <x> <y> [--button left|right|middle] [--double]`
**Actual:** `--double` is not accepted. Exit 2.
**Impact:** Double-click functionality unavailable.

### BUG-006: `--password-stdin` not implemented

**Severity:** MEDIUM
**Spec says:** "Passwords via `--password` flag, `RUSTDESK_PASSWORD` env var, or stdin (`--password-stdin`)"
**Actual:** `--password-stdin` is not a recognized option.
**Impact:** Secure password passing from pipes/scripts not possible.

### BUG-007: `meta` modifier not accepted

**Severity:** MEDIUM
**Spec says:** `--modifiers ctrl,shift,alt,meta`
**Actual:** Only `ctrl`, `shift`, `alt` are accepted. `meta` exits 2.
**Impact:** Cannot send Cmd/Win key combinations.

### BUG-008: `capture` file argument is required (spec says optional)

**Severity:** MEDIUM
**Spec says:** `rustdesk-cli capture [<file.png>]` — brackets mean optional, stdout default
**Actual:** `<FILE>` is required. No stdout piping option.
**Impact:** Cannot pipe screenshots to other tools without writing to disk first.

---

## Minor Issues

### BUG-009: Exit codes don't match SPEC

**Severity:** LOW
**Spec says:**
- `0` — success
- `1` — connection error
- `2` — session error (no active session)
- `3` — input error (bad arguments)

**Actual:** clap uses exit 2 for argument parsing errors. This conflicts with the spec's exit 2 (session error). Input validation errors return exit 2 instead of 3.
**Impact:** Programmatic consumers cannot reliably distinguish session errors from input errors.

### BUG-010: `RUSTDESK_PASSWORD` env var not documented in help

**Severity:** LOW
**Spec mentions:** password can come from `RUSTDESK_PASSWORD` env var
**Actual:** `connect --help` does not mention the env var. The env var *may* be accepted (hard to verify with stubbed connections) but is not discoverable.

### BUG-011: `clipboard set` uses `--text` flag instead of positional arg

**Severity:** CLOSED (design choice)
**PM confirmed:** `--text` flag is the intended design. Not a bug.

### BUG-014: `disconnect` with no session returns exit 0 (should be exit 2)

**Severity:** MEDIUM
**PM says:** disconnect with no session should return exit 2 (connection error)
**Actual:** Returns exit 0
**Impact:** Scripts cannot detect that there was nothing to disconnect from.

---

## PM Answers (2026-03-14)

1. **Q1 (status no session):** exit 0 with state=disconnected. **Current behavior is correct.**
2. **Q2 (disconnect no session):** exit 2 (connection error). **Current behavior is WRONG — returns 0.** → BUG-014
3. **Q3 (do batch failure):** Stop immediately, return error for that step, skip remaining. **Tested — batch doesn't check session at all (BUG-012).**
4. **Q4 (--server vs --id-server):** `--id-server`/`--relay-server` take precedence. `--server` is shorthand. **Not yet testable (connect is partially stubbed).**
5. **Q5 (exec output):** Returns JSON with `stdout`, `stderr`, and `exit_code` fields. **WRONG — uses `output` field, not `stdout`/`stderr`. Also exec is entirely stubbed (BUG-015, BUG-016).**
6. **clipboard set:** `--text` flag confirmed as intended design. BUG-011 closed.

---

## Test Coverage Summary

| Area | Tests | Pass | Fail |
|------|-------|------|------|
| Help/Usage | 17 | 17 | 0 |
| Argument validation | 12 | 12 | 0 |
| No-session errors | 11 | 10 | 1 (disconnect BUG-014) |
| JSON output | 5 | 5 | 0 |
| Spec compliance | 7 | 0 | 7 (missing features) |
| Session lifecycle | 2 | 2 | 0 |
| Invalid server connect | 1 | 1 | 0 |
| Wrong password connect | 1 | 0 | 1 (BUG-003) |
| Batch (do) offline | 2 | 1 | 1 (BUG-012) |
| Error output destination | 2 | 1 | 1 (BUG-013) |
| Env var password | 1 | 1 | 0 |
| Live: connect lifecycle | 13 | 13 | 0 |
| Live: JSON connect/disconnect | 2 | 2 | 0 |
| Live: RUSTDESK_PASSWORD env | 1 | 1 | 0 |
| Live: piped workflow | 11 | 11 | 0 |
| Exec deep tests | 11 | 6 | 5 (BUG-015/016) |
| Clipboard deep tests | 11 | 11 | 0 |
| Shell tests | 3 | 3 | 0 |
| Exec edge cases | 5 | 4 | 1 |
| Batch (do) live | 5 | 5 | 0 |
| Clipboard no session | 2 | 2 | 0 |
| Exec no session | 2 | 2 | 0 |
| JSON consistency (live) | 9 | 8 | 1 |
| Disconnect behavior (PM) | 2 | 0 | 2 (BUG-014) |
| **Total** | **137** | **117** | **20** |

## Bug Summary (Prioritized)

| # | Bug | Severity | Category | Fixed by #18? |
|---|-----|----------|----------|---------------|
| BUG-015 | exec is completely stubbed | CRITICAL | Functionality | **YES** — #18 wires terminal channel through daemon |
| BUG-001 | Connect to invalid server succeeds | CRITICAL | Functionality | **YES** — #18 replaces stub connect with real handshake |
| BUG-002 | Capture reports success, writes no file | CRITICAL | Functionality | **PARTIAL** — real connection needed, but capture pipeline may still be stubbed |
| BUG-003 | Wrong password accepted | MAJOR | Security | **YES** — real auth flow will validate passwords |
| BUG-012 | `do` batch bypasses session check | MAJOR | Functionality | **LIKELY** — real daemon will reject commands on dead connection |
| BUG-013 | Errors go to stdout not stderr | MAJOR | Convention | **NO** — CLI output routing, not daemon issue |
| BUG-014 | Disconnect no-session returns 0, PM says 2 | MEDIUM | Behavior | **MAYBE** — depends on exit code cleanup scope |
| BUG-016 | exec JSON missing stdout/stderr fields | MEDIUM | API | **LIKELY** — new exec implementation should use PM-spec fields |
| BUG-004 | scroll command missing | MEDIUM | Spec gap | **NO** — CLI/clap change, not daemon |
| BUG-005 | click --double missing | MEDIUM | Spec gap | **NO** — CLI/clap change |
| BUG-006 | --password-stdin missing | MEDIUM | Spec gap | **NO** — CLI/clap change |
| BUG-007 | meta modifier missing | MEDIUM | Spec gap | **NO** — CLI/clap change |
| BUG-008 | capture file required (spec says optional) | MEDIUM | Spec gap | **NO** — CLI/clap change |
| BUG-009 | Exit codes don't match spec (2 vs 3) | LOW | Convention | **MAYBE** — if exit code routing is in scope |
| BUG-010 | RUSTDESK_PASSWORD not in help | LOW | Docs | **NO** — help text change |

### Regression test suite: `tests/qa/regression_daemon_wiring.sh`
Targeted tests for verifying #18 fixes. Run after Max's PR lands.
