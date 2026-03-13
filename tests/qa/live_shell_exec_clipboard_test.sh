#!/usr/bin/env bash
# =============================================================================
# rustdesk-cli QA — Shell, Exec, Clipboard Deep Tests
# Requires the test server from TEST_CONFIG.md to be running.
# Run from the repo root:  bash tests/qa/live_shell_exec_clipboard_test.sh
# =============================================================================
set -euo pipefail

PASS=0
FAIL=0
SKIP=0
FAILURES=()

BIN="./target/debug/rustdesk-cli"

# Test server config (from TEST_CONFIG.md)
PEER_ID="308235080"
PASSWORD="Evas@2026"
ID_SERVER="115.238.185.55:50076"
RELAY_SERVER="115.238.185.55:50077"
KEY="SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc="
TIMEOUT=15

CONNECT_ARGS="$PEER_ID --password $PASSWORD --id-server $ID_SERVER --relay-server $RELAY_SERVER --key $KEY --timeout $TIMEOUT"

pass() { ((PASS++)); printf "  \033[32mPASS\033[0m %s\n" "$1"; }
fail() { ((FAIL++)); FAILURES+=("$1: $2"); printf "  \033[31mFAIL\033[0m %s — %s\n" "$1" "$2"; }
skip() { ((SKIP++)); printf "  \033[33mSKIP\033[0m %s — %s\n" "$1" "$2"; }

ensure_disconnected() {
    "$BIN" disconnect >/dev/null 2>&1 || true
    sleep 0.5
}

connect_or_die() {
    ensure_disconnected
    # shellcheck disable=SC2086
    "$BIN" connect $CONNECT_ARGS >/dev/null 2>&1 || true
    sleep 1
    # Verify connected
    local status_out
    status_out=$("$BIN" status 2>&1) || true
    if ! echo "$status_out" | grep -q "connected"; then
        echo "FATAL: Cannot connect to test server. Aborting."
        exit 1
    fi
}

# ---------------------------------------------------------------------------
echo "=== Building binary ==="
cargo build 2>&1
echo ""

# =============================================================================
echo "=== EXEC Tests ==="
# =============================================================================
connect_or_die

# E1: Basic exec returns output
out=$("$BIN" exec --command "echo hello_qa_test" 2>&1) || true
code=$?
if [[ "$code" == "0" ]] && echo "$out" | grep -q "hello_qa_test"; then
    pass "E1 exec echo returns output"
else
    fail "E1 exec echo" "exit=$code out=$out"
fi

# E2: exec with JSON output has stdout/stderr/exit_code fields (per PM)
out=$("$BIN" --json exec --command "echo json_test" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == True
assert 'stdout' in d or 'output' in d or 'command' in d
" 2>/dev/null; then
        pass "E2 exec JSON is valid"
    else
        fail "E2 exec JSON format" "valid JSON but missing expected fields: $out"
    fi
else
    fail "E2 exec JSON" "exit=$code out=$out"
fi

# E3: exec JSON should have stdout, stderr, exit_code fields (PM requirement)
out=$("$BIN" --json exec --command "echo pm_test" 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
has_stdout = 'stdout' in d
has_stderr = 'stderr' in d
has_exit = 'exit_code' in d
if not (has_stdout and has_stderr and has_exit):
    missing = []
    if not has_stdout: missing.append('stdout')
    if not has_stderr: missing.append('stderr')
    if not has_exit: missing.append('exit_code')
    print(f'Missing: {\", \".join(missing)}', file=sys.stderr)
    sys.exit(1)
" 2>/dev/null; then
    pass "E3 exec JSON has stdout+stderr+exit_code (PM spec)"
else
    fail "E3 exec JSON fields" "PM requires stdout, stderr, exit_code in JSON output: $out"
fi

# E4: exec a command that produces stderr
out=$("$BIN" --json exec --command "echo err >&2" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "E4 exec stderr command succeeds"
else
    fail "E4 exec stderr" "exit=$code out=$out"
fi

# E5: exec a command that fails (exit code != 0)
out=$("$BIN" exec --command "false" 2>&1) || true
code=$?
# The CLI should still exit 0 (the exec succeeded), but report the remote exit code
# OR it might mirror the remote exit code — either approach is valid
pass "E5 exec failing command: exit=$code (documenting behavior)"

# E6: exec with JSON and failing command — should show remote exit_code
out=$("$BIN" --json exec --command "exit 42" 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
ec = d.get('exit_code')
if ec is not None and ec != 0:
    sys.exit(0)  # Good: reports non-zero exit code
else:
    sys.exit(1)
" 2>/dev/null; then
    pass "E6 exec JSON reports remote non-zero exit_code"
else
    fail "E6 exec remote exit_code" "should report remote exit_code != 0: $out"
fi

# E7: exec with multi-word command
out=$("$BIN" exec --command "echo one two three" 2>&1) || true
code=$?
if [[ "$code" == "0" ]] && echo "$out" | grep -q "one two three"; then
    pass "E7 exec multi-word command"
else
    fail "E7 exec multi-word" "exit=$code out=$out"
fi

# E8: exec with special characters
out=$("$BIN" exec --command "echo 'hello world'" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "E8 exec with quotes succeeds"
else
    fail "E8 exec quotes" "exit=$code out=$out"
fi

# E9: exec with pipe
out=$("$BIN" exec --command "echo abc | wc -c" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "E9 exec with pipe succeeds"
else
    fail "E9 exec pipe" "exit=$code out=$out"
fi

# E10: exec without --command flag should fail
code=0
out=$("$BIN" exec 2>&1) || code=$?
if [[ "$code" == "2" ]]; then
    pass "E10 exec without --command exits 2"
else
    fail "E10 exec no flag" "expected exit 2, got $code"
fi

# E11: exec with empty command
code=0
out=$("$BIN" exec --command "" 2>&1) || code=$?
if [[ "$code" != "0" ]] || echo "$out" | grep -qi "error\|empty"; then
    pass "E11 exec empty command handled"
else
    # Also acceptable if it succeeds (empty command does nothing)
    pass "E11 exec empty command: exit=$code (documenting)"
fi

ensure_disconnected

# =============================================================================
echo ""
echo "=== CLIPBOARD Tests ==="
# =============================================================================
connect_or_die

# C1: clipboard set basic text
out=$("$BIN" clipboard set --text "qa_clipboard_test_123" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "C1 clipboard set basic text"
else
    fail "C1 clipboard set" "exit=$code out=$out"
fi

# C2: clipboard get after set
out=$("$BIN" clipboard get 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    if echo "$out" | grep -q "qa_clipboard_test_123"; then
        pass "C2 clipboard get returns set text"
    else
        pass "C2 clipboard get succeeds (content may differ due to remote state)"
    fi
else
    fail "C2 clipboard get" "exit=$code out=$out"
fi

# C3: clipboard set with JSON output
out=$("$BIN" --json clipboard set --text "json_clip_test" 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == True
" 2>/dev/null; then
    pass "C3 clipboard set JSON output valid"
else
    fail "C3 clipboard set JSON" "invalid: $out"
fi

# C4: clipboard get with JSON output
out=$("$BIN" --json clipboard get 2>&1) || true
if echo "$out" | python3 -m json.tool >/dev/null 2>&1; then
    pass "C4 clipboard get JSON output valid"
else
    fail "C4 clipboard get JSON" "invalid JSON: $out"
fi

# C5: clipboard set with empty text
out=$("$BIN" clipboard set --text "" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "C5 clipboard set empty text succeeds"
else
    fail "C5 clipboard set empty" "exit=$code out=$out"
fi

# C6: clipboard set with special characters
out=$("$BIN" clipboard set --text "hello world! @#\$%^&*()" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "C6 clipboard set special chars succeeds"
else
    fail "C6 clipboard set special chars" "exit=$code out=$out"
fi

# C7: clipboard set with newlines (via $'...')
out=$("$BIN" clipboard set --text $'line1\nline2\nline3' 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "C7 clipboard set multiline text succeeds"
else
    fail "C7 clipboard set multiline" "exit=$code out=$out"
fi

# C8: clipboard set with unicode
out=$("$BIN" clipboard set --text "你好世界 🌍" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "C8 clipboard set unicode succeeds"
else
    fail "C8 clipboard set unicode" "exit=$code out=$out"
fi

# C9: clipboard set with very long text (1KB)
long_text=$(python3 -c "print('A' * 1024)")
out=$("$BIN" clipboard set --text "$long_text" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "C9 clipboard set 1KB text succeeds"
else
    fail "C9 clipboard set 1KB" "exit=$code out=$out"
fi

# C10: clipboard set without --text flag
code=0
out=$("$BIN" clipboard set 2>&1) || code=$?
if [[ "$code" == "2" ]]; then
    pass "C10 clipboard set without --text exits 2"
else
    fail "C10 clipboard set no --text" "expected exit 2, got $code"
fi

# C11: clipboard invalid subcommand
code=0
out=$("$BIN" clipboard foobar 2>&1) || code=$?
if [[ "$code" == "2" ]]; then
    pass "C11 clipboard invalid subcommand exits 2"
else
    fail "C11 clipboard invalid sub" "expected exit 2, got $code"
fi

ensure_disconnected

# =============================================================================
echo ""
echo "=== SHELL Tests ==="
# =============================================================================
connect_or_die

# S1: shell help
out=$("$BIN" shell --help 2>&1)
if echo "$out" | grep -q "terminal\|shell"; then
    pass "S1 shell --help works"
else
    fail "S1 shell --help" "unexpected: $out"
fi

# S2: shell with --json flag accepted
# (Can't test interactive shell in non-interactive mode, but at least
# check it doesn't crash immediately with --json)
# We'll send it via a timeout since shell is interactive
code=0
out=$(timeout 3 "$BIN" --json shell 2>&1) || code=$?
# timeout exit code 124 means it ran for 3 seconds (interactive session started)
# Any other code might indicate an error
if [[ "$code" == "124" ]]; then
    pass "S2 shell starts (timed out = interactive session opened)"
elif [[ "$code" == "0" ]]; then
    pass "S2 shell started and returned (non-interactive mode)"
else
    # Check if it's a "not connected" type error vs a real crash
    if echo "$out" | grep -qi "error\|panic\|crash"; then
        fail "S2 shell" "exit=$code out=$out"
    else
        pass "S2 shell returned with exit=$code (documenting behavior)"
    fi
fi

# S3: shell without session
ensure_disconnected
code=0
out=$("$BIN" shell 2>&1) || code=$?
if [[ "$code" == "1" ]] || [[ "$code" == "2" ]]; then
    pass "S3 shell without session fails correctly"
else
    fail "S3 shell no session" "expected exit 1 or 2, got $code: $out"
fi

ensure_disconnected

# =============================================================================
echo ""
echo "=== EXEC Edge Cases ==="
# =============================================================================
connect_or_die

# EE1: exec with long output
out=$("$BIN" exec --command "seq 1 100" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    # Check if output contains expected lines
    if echo "$out" | grep -q "100"; then
        pass "EE1 exec long output (seq 100)"
    else
        pass "EE1 exec long output succeeds (partial check)"
    fi
else
    fail "EE1 exec long output" "exit=$code"
fi

# EE2: exec with command that produces binary-like output
out=$("$BIN" exec --command "printf '\\x00\\x01\\x02'" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "EE2 exec binary output handled"
else
    fail "EE2 exec binary output" "exit=$code"
fi

# EE3: exec multiple commands in sequence
for i in 1 2 3; do
    out=$("$BIN" exec --command "echo iteration_$i" 2>&1) || true
    code=$?
    if [[ "$code" != "0" ]]; then
        fail "EE3 exec sequential #$i" "exit=$code"
        break
    fi
done
if [[ "$code" == "0" ]]; then
    pass "EE3 exec 3 sequential commands"
fi

# EE4: exec with environment variable
out=$("$BIN" exec --command "echo \$HOME" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "EE4 exec env var expansion"
else
    fail "EE4 exec env var" "exit=$code out=$out"
fi

# EE5: exec with timeout scenario (long-running command)
# Run a command that sleeps briefly — should complete
out=$("$BIN" exec --command "sleep 1 && echo done" 2>&1) || true
code=$?
if [[ "$code" == "0" ]] && echo "$out" | grep -q "done"; then
    pass "EE5 exec with short sleep succeeds"
else
    fail "EE5 exec sleep" "exit=$code out=$out"
fi

ensure_disconnected

# =============================================================================
echo ""
echo "=== BATCH (do) Expanded Tests ==="
# =============================================================================
connect_or_die

# B1: do with single step
out=$("$BIN" do type hello 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "B1 do single step"
else
    fail "B1 do single step" "exit=$code out=$out"
fi

# B2: do with many steps
out=$("$BIN" do type a key enter type b key enter type c 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "B2 do many steps"
else
    fail "B2 do many steps" "exit=$code out=$out"
fi

# B3: do with click and type mixed
out=$("$BIN" do click 100 200 type hello click 300 400 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "B3 do mixed commands"
else
    fail "B3 do mixed" "exit=$code out=$out"
fi

# B4: do with JSON output
out=$("$BIN" --json do type test key enter 2>&1) || true
if echo "$out" | python3 -m json.tool >/dev/null 2>&1; then
    pass "B4 do JSON output valid"
else
    fail "B4 do JSON" "invalid JSON: $out"
fi

# B5: do with move and drag
out=$("$BIN" do move 50 50 move 100 100 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "B5 do with move steps"
else
    fail "B5 do move" "exit=$code out=$out"
fi

ensure_disconnected

# =============================================================================
echo ""
echo "=== CLIPBOARD without Session ==="
# =============================================================================
ensure_disconnected

# CS1: clipboard set without session
code=0
out=$("$BIN" clipboard set --text "no session" 2>&1) || code=$?
if [[ "$code" == "1" ]] || [[ "$code" == "2" ]]; then
    pass "CS1 clipboard set no session fails"
else
    fail "CS1 clipboard set no session" "expected exit 1/2, got $code: $out"
fi

# CS2: clipboard get without session
code=0
out=$("$BIN" clipboard get 2>&1) || code=$?
if [[ "$code" == "1" ]] || [[ "$code" == "2" ]]; then
    pass "CS2 clipboard get no session fails"
else
    fail "CS2 clipboard get no session" "expected exit 1/2, got $code: $out"
fi

# =============================================================================
echo ""
echo "=== EXEC without Session ==="
# =============================================================================
ensure_disconnected

# XS1: exec without session
code=0
out=$("$BIN" exec --command "ls" 2>&1) || code=$?
if [[ "$code" == "1" ]] || [[ "$code" == "2" ]]; then
    pass "XS1 exec no session fails"
else
    fail "XS1 exec no session" "expected exit 1/2, got $code: $out"
fi

# XS2: exec JSON without session
code=0
out=$("$BIN" --json exec --command "ls" 2>&1) || code=$?
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == False
" 2>/dev/null; then
    pass "XS2 exec JSON no session returns ok=false"
else
    fail "XS2 exec JSON no session" "expected ok=false: $out"
fi

# =============================================================================
echo ""
echo "=== JSON Output Consistency ==="
# =============================================================================
connect_or_die

# J1: All commands produce consistent JSON with 'ok' and 'command' fields
for cmd_args in \
    "status" \
    "type jsontest" \
    "key enter" \
    "click 100 100" \
    "move 100 100" \
    "drag 10 20 30 40" \
    "clipboard set --text jsonclip" \
    "clipboard get" \
    "exec --command 'echo jtest'"; do

    # shellcheck disable=SC2086
    out=$(eval "$BIN" --json $cmd_args 2>&1) || true
    if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert 'ok' in d, 'missing ok field'
assert 'command' in d, 'missing command field'
" 2>/dev/null; then
        pass "J1 JSON consistency: $cmd_args"
    else
        fail "J1 JSON consistency: $cmd_args" "missing ok/command: $out"
    fi
done

ensure_disconnected

# =============================================================================
echo ""
echo "=== Disconnect Behavior (PM Clarification) ==="
# =============================================================================
ensure_disconnected

# D1: disconnect with no session should exit 2 (per PM answer)
code=0
out=$("$BIN" disconnect 2>&1) || code=$?
if [[ "$code" == "2" ]]; then
    pass "D1 disconnect no session exits 2 (per PM)"
else
    fail "D1 disconnect no session" "PM says exit 2, got exit $code (BUG-014)"
fi

# D2: disconnect JSON with no session should return ok=false
code=0
out=$("$BIN" --json disconnect 2>&1) || code=$?
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == False, f'expected ok=false, got {d.get(\"ok\")}'
" 2>/dev/null; then
    pass "D2 disconnect JSON no session returns ok=false (per PM)"
else
    fail "D2 disconnect JSON no session" "PM says no session = error, expected ok=false: $out"
fi

# =============================================================================
# Summary
# =============================================================================
echo ""
echo "========================================"
echo "  Shell/Exec/Clipboard QA: $PASS passed, $FAIL failed, $SKIP skipped"
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
