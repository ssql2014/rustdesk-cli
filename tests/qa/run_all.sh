#!/usr/bin/env bash
# =============================================================================
# rustdesk-cli QA Test Runner
# Black-box tests exercising the CLI binary from the outside.
# Run from the repo root:  bash tests/qa/run_all.sh
# =============================================================================
set -euo pipefail

PASS=0
FAIL=0
SKIP=0
FAILURES=()

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
BIN="./target/debug/rustdesk-cli"

pass() { ((PASS++)); printf "  \033[32mPASS\033[0m %s\n" "$1"; }
fail() { ((FAIL++)); FAILURES+=("$1: $2"); printf "  \033[31mFAIL\033[0m %s — %s\n" "$1" "$2"; }
skip() { ((SKIP++)); printf "  \033[33mSKIP\033[0m %s — %s\n" "$1" "$2"; }

# Run a command and capture stdout+stderr and exit code
run() {
    local out
    out=$("$@" 2>&1) || true
    echo "$out"
}

run_exit() {
    "$@" >/dev/null 2>&1
    echo $?
}

ensure_disconnected() {
    "$BIN" disconnect >/dev/null 2>&1 || true
}

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "=== Building binary ==="
cargo build 2>&1
echo ""

# Make sure no stale session
ensure_disconnected

# =============================================================================
echo "=== Suite 1: Help & Usage ==="
# =============================================================================

# T1.1: No args shows help and exits 2
out=$(run "$BIN")
code=$(run_exit "$BIN")
if [[ "$code" == "2" ]] && echo "$out" | grep -q "Usage:"; then
    pass "T1.1 no-args shows usage, exit 2"
else
    fail "T1.1 no-args" "expected exit 2 + Usage, got exit=$code"
fi

# T1.2: --help exits 0
code=$(run_exit "$BIN" --help)
if [[ "$code" == "0" ]]; then
    pass "T1.2 --help exits 0"
else
    fail "T1.2 --help" "expected exit 0, got $code"
fi

# T1.3: All expected subcommands present in help
out=$(run "$BIN" --help)
for cmd in connect disconnect shell exec clipboard status capture type key click move drag do; do
    if echo "$out" | grep -q "$cmd"; then
        pass "T1.3 help lists '$cmd'"
    else
        fail "T1.3 help lists '$cmd'" "subcommand missing from help output"
    fi
done

# T1.4: --json flag present in help
if echo "$out" | grep -q "\-\-json"; then
    pass "T1.4 --json in help"
else
    fail "T1.4 --json in help" "flag missing"
fi

# T1.5: Unknown subcommand
out=$(run "$BIN" foobar)
code=$(run_exit "$BIN" foobar)
if [[ "$code" == "2" ]]; then
    pass "T1.5 unknown subcommand exits 2"
else
    fail "T1.5 unknown subcommand" "expected exit 2, got $code"
fi

# =============================================================================
echo ""
echo "=== Suite 2: Connect Argument Validation ==="
# =============================================================================

# T2.1: connect with no ID
code=$(run_exit "$BIN" connect)
if [[ "$code" == "2" ]]; then
    pass "T2.1 connect no ID exits 2"
else
    fail "T2.1 connect no ID" "expected exit 2, got $code"
fi

# T2.2: connect help shows all expected options
out=$(run "$BIN" connect --help)
for opt in password server id-server relay-server key timeout; do
    if echo "$out" | grep -qi "$opt"; then
        pass "T2.2 connect --help shows --$opt"
    else
        fail "T2.2 connect --help shows --$opt" "option missing"
    fi
done

# T2.3: connect --timeout default is 15
out=$(run "$BIN" connect --help)
if echo "$out" | grep -q "default: 15"; then
    pass "T2.3 connect --timeout default is 15"
else
    fail "T2.3 connect --timeout default" "expected default: 15 in help"
fi

# =============================================================================
echo ""
echo "=== Suite 3: Commands Without Session (No Active Connection) ==="
# =============================================================================
ensure_disconnected

# Commands that require an active session should fail with exit 1 and clear error
for cmd_args in \
    "type hello" \
    "key enter" \
    "click 500 300" \
    "move 100 200" \
    "drag 10 20 30 40" \
    "exec --command ls" \
    "capture /tmp/test_qa.png" \
    "clipboard get" \
    "shell"; do

    # shellcheck disable=SC2086
    out=$(run "$BIN" $cmd_args)
    # shellcheck disable=SC2086
    code=$(run_exit "$BIN" $cmd_args)
    if [[ "$code" == "1" ]] || [[ "$code" == "2" ]]; then
        if echo "$out" | grep -iq "no active session\|not connected\|connect first"; then
            pass "T3 no-session: $cmd_args → error message + exit $code"
        else
            fail "T3 no-session: $cmd_args" "exit $code but unclear error: $out"
        fi
    else
        fail "T3 no-session: $cmd_args" "expected exit 1 or 2, got $code"
    fi
done

# T3.special: status with no session should still work (exit 0, shows disconnected)
out=$(run "$BIN" status)
code=$(run_exit "$BIN" status)
if [[ "$code" == "0" ]] && echo "$out" | grep -qi "disconnected\|connected.*false"; then
    pass "T3 status no-session: exits 0, shows disconnected"
else
    fail "T3 status no-session" "expected exit 0 + disconnected, got exit=$code out=$out"
fi

# T3.special: disconnect with no session is idempotent (exit 0)
code=$(run_exit "$BIN" disconnect)
if [[ "$code" == "0" ]]; then
    pass "T3 disconnect no-session: idempotent exit 0"
else
    fail "T3 disconnect no-session" "expected exit 0, got $code"
fi

# =============================================================================
echo ""
echo "=== Suite 4: JSON Output Mode ==="
# =============================================================================
ensure_disconnected

# T4.1: --json status (no session) outputs valid JSON
out=$(run "$BIN" --json status)
if echo "$out" | python3 -m json.tool >/dev/null 2>&1; then
    pass "T4.1 --json status outputs valid JSON"
else
    fail "T4.1 --json status" "invalid JSON: $out"
fi

# T4.2: JSON status has expected fields
if echo "$out" | python3 -c "import sys,json; d=json.load(sys.stdin); assert 'ok' in d and 'command' in d" 2>/dev/null; then
    pass "T4.2 --json status has 'ok' and 'command' fields"
else
    fail "T4.2 --json status fields" "missing ok/command in: $out"
fi

# T4.3: --json disconnect outputs valid JSON
out=$(run "$BIN" --json disconnect)
if echo "$out" | python3 -m json.tool >/dev/null 2>&1; then
    pass "T4.3 --json disconnect outputs valid JSON"
else
    fail "T4.3 --json disconnect" "invalid JSON: $out"
fi

# T4.4: --json error outputs valid JSON with error info
out=$(run "$BIN" --json type hello)
if echo "$out" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get('ok')==False and 'error' in d" 2>/dev/null; then
    pass "T4.4 --json error has ok=false + error"
else
    fail "T4.4 --json error format" "bad error JSON: $out"
fi

# T4.5: subcommand-level --json also works (e.g. `rustdesk-cli status --json`)
out=$(run "$BIN" status --json)
if echo "$out" | python3 -m json.tool >/dev/null 2>&1; then
    pass "T4.5 subcommand-level --json works"
else
    fail "T4.5 subcommand-level --json" "invalid JSON: $out"
fi

# =============================================================================
echo ""
echo "=== Suite 5: Input Validation (Bad Arguments) ==="
# =============================================================================

# T5.1: click with non-numeric X
out=$(run "$BIN" click abc 100)
code=$(run_exit "$BIN" click abc 100)
if [[ "$code" == "2" ]]; then
    pass "T5.1 click non-numeric X exits 2"
else
    fail "T5.1 click non-numeric X" "expected exit 2, got $code"
fi

# T5.2: click with missing Y
code=$(run_exit "$BIN" click 100)
if [[ "$code" == "2" ]]; then
    pass "T5.2 click missing Y exits 2"
else
    fail "T5.2 click missing Y" "expected exit 2, got $code"
fi

# T5.3: click with invalid button
code=$(run_exit "$BIN" click 100 200 --button foobar)
if [[ "$code" == "2" ]]; then
    pass "T5.3 click invalid button exits 2"
else
    fail "T5.3 click invalid button" "expected exit 2, got $code"
fi

# T5.4: key with invalid modifier
code=$(run_exit "$BIN" key enter --modifiers badmod)
if [[ "$code" == "2" ]]; then
    pass "T5.4 key invalid modifier exits 2"
else
    fail "T5.4 key invalid modifier" "expected exit 2, got $code"
fi

# T5.5: drag with only 3 of 4 coordinates
code=$(run_exit "$BIN" drag 10 20 30)
if [[ "$code" == "2" ]]; then
    pass "T5.5 drag missing coordinate exits 2"
else
    fail "T5.5 drag missing coordinate" "expected exit 2, got $code"
fi

# T5.6: capture quality out of range (0 and 101)
code=$(run_exit "$BIN" capture /tmp/test.jpg --format jpg --quality 0)
if [[ "$code" == "2" ]]; then
    pass "T5.6a capture quality=0 exits 2"
else
    fail "T5.6a capture quality=0" "expected exit 2, got $code"
fi

code=$(run_exit "$BIN" capture /tmp/test.jpg --format jpg --quality 101)
if [[ "$code" == "2" ]]; then
    pass "T5.6b capture quality=101 exits 2"
else
    fail "T5.6b capture quality=101" "expected exit 2, got $code"
fi

# T5.7: capture --format with invalid format
out=$(run "$BIN" capture /tmp/test.bmp --format bmp)
code=$(run_exit "$BIN" capture /tmp/test.bmp --format bmp)
if [[ "$code" == "2" ]]; then
    pass "T5.7 capture invalid format exits 2"
else
    fail "T5.7 capture invalid format" "expected exit 2, got $code"
fi

# T5.8: exec without --command
code=$(run_exit "$BIN" exec)
if [[ "$code" == "2" ]]; then
    pass "T5.8 exec without --command exits 2"
else
    fail "T5.8 exec without --command" "expected exit 2, got $code"
fi

# T5.9: clipboard without subcommand
code=$(run_exit "$BIN" clipboard)
if [[ "$code" == "2" ]]; then
    pass "T5.9 clipboard no subcommand exits 2"
else
    fail "T5.9 clipboard no subcommand" "expected exit 2, got $code"
fi

# T5.10: clipboard set without --text
code=$(run_exit "$BIN" clipboard set)
if [[ "$code" == "2" ]]; then
    pass "T5.10 clipboard set without --text exits 2"
else
    fail "T5.10 clipboard set without --text" "expected exit 2, got $code"
fi

# =============================================================================
echo ""
echo "=== Suite 6: Spec Compliance Checks ==="
# =============================================================================

# T6.1: SPEC says `scroll <x> <y> <delta>` should exist
out=$(run "$BIN" --help)
if echo "$out" | grep -q "scroll"; then
    pass "T6.1 scroll command exists (per spec)"
else
    fail "T6.1 scroll command missing" "SPEC.md requires 'scroll <x> <y> <delta>' but it is not implemented"
fi

# T6.2: SPEC says click --double should exist
out=$(run "$BIN" click --help)
if echo "$out" | grep -q "\-\-double"; then
    pass "T6.2 click --double flag exists (per spec)"
else
    fail "T6.2 click --double missing" "SPEC.md requires '--double' flag but it is not implemented"
fi

# T6.3: SPEC says --password-stdin should exist
out=$(run "$BIN" connect --help)
if echo "$out" | grep -q "password-stdin"; then
    pass "T6.3 --password-stdin exists (per spec)"
else
    fail "T6.3 --password-stdin missing" "SPEC.md security section requires '--password-stdin' but it is not implemented"
fi

# T6.4: SPEC says key modifiers include 'meta'
out=$(run "$BIN" key --help)
if echo "$out" | grep -q "meta"; then
    pass "T6.4 key --modifiers includes 'meta' (per spec)"
else
    fail "T6.4 key --modifiers missing 'meta'" "SPEC.md says '--modifiers ctrl,shift,alt,meta' but 'meta' is not accepted"
fi

# T6.5: SPEC says capture file is optional (stdout default)
out=$(run "$BIN" capture --help)
if echo "$out" | grep -q "optional\|stdout\|\[FILE\]\|\[<file"; then
    pass "T6.5 capture file is optional (per spec)"
else
    fail "T6.5 capture file is required" "SPEC.md says 'capture [<file.png>]' (optional) but CLI requires <FILE>"
fi

# T6.6: SPEC exit codes: 0=success, 1=connection, 2=session, 3=input
# clap uses exit 2 for parse errors — SPEC says 3 for input errors
# This is a design conflict worth noting
out=$(run "$BIN" click abc 100)
code=$(run_exit "$BIN" click abc 100)
if [[ "$code" == "3" ]]; then
    pass "T6.6 input error exit code is 3 (per spec)"
else
    fail "T6.6 input error exit code" "SPEC.md says exit 3 for 'input error (bad arguments)' but got exit $code"
fi

# T6.7: SPEC says RUSTDESK_PASSWORD env var should work
# (Can't fully test without a valid server, but connect help should mention it)
out=$(run "$BIN" connect --help)
if echo "$out" | grep -qi "RUSTDESK_PASSWORD\|env"; then
    pass "T6.7 RUSTDESK_PASSWORD documented in help"
else
    fail "T6.7 RUSTDESK_PASSWORD not in help" "SPEC.md mentions env var but help doesn't document it"
fi

# =============================================================================
echo ""
echo "=== Suite 7: Session Lifecycle ==="
# =============================================================================
ensure_disconnected

# T7.1: Lock file is gone when disconnected
if [[ ! -f /tmp/rustdesk-cli.lock ]]; then
    pass "T7.1 no lock file when disconnected"
else
    fail "T7.1 stale lock file" "lock file exists when no session is active"
fi

# T7.2: Socket is gone when disconnected
if [[ ! -S /tmp/rustdesk-cli.sock ]]; then
    pass "T7.2 no socket when disconnected"
else
    fail "T7.2 stale socket" "socket exists when no session is active"
fi

# =============================================================================
echo ""
echo "=== Suite 8: Connect to Invalid/Unreachable Server ==="
# =============================================================================
ensure_disconnected

# T8.1: Connect to an invalid server should fail (NOT succeed with fake data)
out=$(run "$BIN" connect 999999999 --server invalid.example.com --timeout 3)
code=$(run_exit "$BIN" connect 999999999 --server invalid.example.com --timeout 3)
# Clean up in case it "succeeded"
ensure_disconnected
if [[ "$code" != "0" ]]; then
    pass "T8.1 connect to invalid server fails"
else
    fail "T8.1 connect to invalid server SUCCEEDS" "BUG: exit 0 with output: $out — connection to nonexistent server should fail"
fi

# T8.2: Connect with wrong password should fail
ensure_disconnected
out=$("$BIN" connect 308235080 \
    --password WRONG_PASSWORD \
    --id-server 115.238.185.55:50076 \
    --relay-server 115.238.185.55:50077 \
    --key SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc= \
    --timeout 10 2>&1) || true
code=$?
ensure_disconnected
if [[ "$code" != "0" ]]; then
    pass "T8.2 connect with wrong password fails"
else
    if echo "$out" | grep -qi "auth\|password\|denied\|failed"; then
        fail "T8.2 wrong password" "exit 0 but error message present: $out"
    else
        fail "T8.2 wrong password SUCCEEDS" "BUG: exit 0, output: $out — wrong password should fail"
    fi
fi

# =============================================================================
echo ""
echo "=== Suite 9: Batch (do) Command ==="
# =============================================================================
ensure_disconnected

# T9.1: `do` with no steps
out=$(run "$BIN" do)
code=$(run_exit "$BIN" do)
if [[ "$code" == "2" ]]; then
    pass "T9.1 do with no steps exits 2"
else
    fail "T9.1 do with no steps" "expected exit 2, got $code"
fi

# T9.2: `do` with session-requiring commands but no session
out=$(run "$BIN" do type hello key enter)
code=$(run_exit "$BIN" do type hello key enter)
if [[ "$code" != "0" ]]; then
    pass "T9.2 do without session fails"
else
    fail "T9.2 do without session" "expected failure, got exit 0: $out"
fi

# =============================================================================
echo ""
echo "=== Suite 10: Error Output Destination ==="
# =============================================================================
ensure_disconnected

# T10.1: Error messages go to stderr (per SPEC)
stdout_out=$("$BIN" type hello 2>/dev/null) || true
stderr_out=$("$BIN" type hello 2>&1 >/dev/null) || true
if [[ -z "$stdout_out" ]] && [[ -n "$stderr_out" ]]; then
    pass "T10.1 errors go to stderr (not stdout)"
elif [[ -n "$stdout_out" ]]; then
    fail "T10.1 errors on stdout" "SPEC says errors go to stderr, but got stdout: $stdout_out"
else
    fail "T10.1 no error output at all" "expected error on stderr but got nothing"
fi

# T10.2: JSON errors go to stdout (expected for machine parsing)
stdout_json=$("$BIN" --json type hello 2>/dev/null) || true
if echo "$stdout_json" | python3 -m json.tool >/dev/null 2>&1; then
    pass "T10.2 JSON errors go to stdout"
else
    fail "T10.2 JSON errors" "expected JSON on stdout, got: $stdout_json"
fi

# =============================================================================
echo ""
echo "=== Suite 11: RUSTDESK_PASSWORD Environment Variable ==="
# =============================================================================
ensure_disconnected

# T11.1: RUSTDESK_PASSWORD should be picked up when --password is not given
# We test with an invalid server so it will try to connect (proving it read the password)
out=$(RUSTDESK_PASSWORD=envtest "$BIN" connect 999999999 --server invalid.example.com --timeout 3 2>&1) || true
code=$?
ensure_disconnected
# We can't easily distinguish "password read but connection failed" from "password not read"
# But at minimum, the binary should not crash or demand --password
if echo "$out" | grep -qi "required.*password\|missing.*password"; then
    fail "T11.1 RUSTDESK_PASSWORD" "env var not picked up; CLI demanded --password"
else
    pass "T11.1 RUSTDESK_PASSWORD accepted (no --password required)"
fi

# =============================================================================
# Summary
# =============================================================================
echo ""
echo "========================================"
echo "  QA Results: $PASS passed, $FAIL failed, $SKIP skipped"
echo "========================================"
if [[ ${#FAILURES[@]} -gt 0 ]]; then
    echo ""
    echo "FAILURES:"
    for f in "${FAILURES[@]}"; do
        echo "  - $f"
    done
fi
echo ""
exit "$FAIL"
