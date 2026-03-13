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

**Severity:** LOW (design choice)
**Spec suggests:** `clipboard set "<text>"` (positional)
**Actual:** Requires `clipboard set --text "<text>"` (named flag)
**Impact:** Minor ergonomic difference. Running `clipboard set "hello"` fails with "unexpected argument".

---

## Unclear Requirements (Questions for Ada)

1. **Q1:** Should `status` with no active session return exit 0 or exit 2? Currently returns exit 0 with "disconnected". Spec says exit 2 for "no active session". But showing disconnected status isn't really an error — it's valid information. Recommend: keep exit 0.

2. **Q2:** Should `disconnect` with no active session return exit 0 or exit 2? Currently exit 0 (idempotent). Spec could be read either way. Recommend: keep exit 0 (idempotent is safer for scripting).

3. **Q3:** The `do` (batch) command — what's the expected behavior when one step fails mid-batch? Stop immediately? Continue? Report partial results?

4. **Q4:** The `--server` flag sets a combined server address. But `--id-server` and `--relay-server` override individually. What happens when `--server` and `--id-server` are both specified? Which wins?

5. **Q5:** `exec --command` — should this return the command's stdout? Its exit code? Both? Is there a timeout?

---

## Test Coverage Summary

| Area | Tests | Status |
|------|-------|--------|
| Help/Usage | 17 | All pass |
| Argument validation | 12 | All pass |
| No-session errors | 11 | All pass |
| JSON output | 5 | All pass |
| Spec compliance | 7 | 6 FAIL (missing features) |
| Session lifecycle | 2 | Pass |
| Invalid server | 1 | Needs retest (inconsistent) |
| Wrong password | 1 | FAIL (BUG-003) |
| Batch (do) | 2 | 1 FAIL (BUG-012) |
| Error output dest | 2 | 1 FAIL (BUG-013) |
| Env var password | 1 | Pass (accepted) |

**Overall: 5 critical/major bugs, 5 missing spec features, 3 minor issues, 5 open questions.**
