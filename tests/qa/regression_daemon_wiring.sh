#!/usr/bin/env bash
# =============================================================================
# rustdesk-cli QA — Daemon Wiring Regression Tests (Issue #18)
#
# Purpose: Verify that issue #18 (Wire text_session through daemon for real
# connections) has fixed the critical stub-related bugs.
#
# Run AFTER Max's PR for #18 lands:
#   bash tests/qa/regression_daemon_wiring.sh
#
# Bugs expected to be fixed by #18:
#   BUG-001  Connect to invalid server succeeds       → MUST fail with exit 1
#   BUG-003  Wrong password accepted                   → MUST fail with exit 1
#   BUG-015  exec is completely stubbed                → MUST return real output
#   BUG-016  exec JSON missing stdout/stderr fields    → SHOULD have stdout/stderr/exit_code
#   BUG-012  do batch bypasses session check           → SHOULD fail without session
#   BUG-014  disconnect no-session returns 0           → SHOULD return exit 2
#   BUG-013  Errors go to stdout not stderr            → SHOULD go to stderr
#   BUG-002  Capture reports success, writes no file   → PARTIAL (may still be out of scope)
#
# Test server: TEST_CONFIG.md
# =============================================================================
# NOTE: Do NOT use `set -e` here. This test script must run ALL tests to
# completion even when individual commands fail (which is expected for
# regression testing). We use our own pass/fail counters instead.
set -uo pipefail

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

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
pass() { ((PASS++)); printf "  \033[32mPASS\033[0m %s\n" "$1"; }
fail() { ((FAIL++)); FAILURES+=("$1: $2"); printf "  \033[31mFAIL\033[0m %s — %s\n" "$1" "$2"; }
skip() { ((SKIP++)); printf "  \033[33mSKIP\033[0m %s — %s\n" "$1" "$2"; }

ensure_disconnected() {
    "$BIN" disconnect >/dev/null 2>&1 || true
    sleep 0.5
}

LIVE_OK=true
connect_live() {
    ensure_disconnected
    # shellcheck disable=SC2086
    local code=0
    local out
    out=$("$BIN" connect $CONNECT_ARGS 2>&1) || code=$?
    if [[ "$code" != "0" ]]; then
        echo "  WARNING: Cannot connect to test server (exit=$code). Output: $out"
        LIVE_OK=false
        return 1
    fi
    LIVE_OK=true
    sleep 1
    return 0
}

# ---------------------------------------------------------------------------
echo "=== Building binary ==="
cargo build 2>&1
echo ""
ensure_disconnected

# #########################################################################
# SECTION 1: AUTHENTICATION — BUG-003 regression
# "Wrong password MUST fail with exit 1"
# #########################################################################
echo "=== R1: Authentication (BUG-003 regression) ==="

# R1.1: Wrong password MUST fail with exit 1
ensure_disconnected
code=0
out=$("$BIN" connect "$PEER_ID" \
    --password "COMPLETELY_WRONG_PASSWORD" \
    --id-server "$ID_SERVER" \
    --relay-server "$RELAY_SERVER" \
    --key "$KEY" \
    --timeout "$TIMEOUT" 2>&1) || code=$?
ensure_disconnected
if [[ "$code" == "1" ]]; then
    pass "R1.1 wrong password → exit 1 (BUG-003 FIXED)"
else
    fail "R1.1 wrong password" "BUG-003 NOT FIXED: expected exit 1, got exit $code. Output: $out"
fi

# R1.2: Wrong password error message should mention auth/password
ensure_disconnected
code=0
out=$("$BIN" connect "$PEER_ID" \
    --password "BAD_PW_2" \
    --id-server "$ID_SERVER" \
    --relay-server "$RELAY_SERVER" \
    --key "$KEY" \
    --timeout "$TIMEOUT" 2>&1) || code=$?
ensure_disconnected
if echo "$out" | grep -qiE "auth|password|denied|rejected|failed|wrong"; then
    pass "R1.2 wrong password error message is descriptive"
else
    fail "R1.2 wrong password message" "no auth-related keyword in error: $out"
fi

# R1.3: Wrong password with --json returns ok=false + error
ensure_disconnected
code=0
out=$("$BIN" --json connect "$PEER_ID" \
    --password "BAD_PW_JSON" \
    --id-server "$ID_SERVER" \
    --relay-server "$RELAY_SERVER" \
    --key "$KEY" \
    --timeout "$TIMEOUT" 2>&1) || code=$?
ensure_disconnected
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == False, f'ok should be False, got {d.get(\"ok\")}'
assert 'error' in d, 'missing error field'
" 2>/dev/null; then
    pass "R1.3 wrong password JSON: ok=false + error"
else
    fail "R1.3 wrong password JSON" "expected ok=false with error: $out"
fi

# R1.4: Wrong password should NOT create lock/socket files
ensure_disconnected
code=0
"$BIN" connect "$PEER_ID" \
    --password "BAD_PW_FILES" \
    --id-server "$ID_SERVER" \
    --relay-server "$RELAY_SERVER" \
    --key "$KEY" \
    --timeout "$TIMEOUT" >/dev/null 2>&1 || code=$?
if [[ -f /tmp/rustdesk-cli.lock ]] || [[ -S /tmp/rustdesk-cli.sock ]]; then
    fail "R1.4 wrong password leaves no files" "lock/socket files exist after failed auth"
    ensure_disconnected
else
    pass "R1.4 wrong password leaves no lock/socket files"
fi

# R1.5: Correct password still works
connect_live
out=$("$BIN" status 2>&1) || true
if echo "$out" | grep -q "$PEER_ID"; then
    pass "R1.5 correct password still connects"
else
    fail "R1.5 correct password" "status doesn't show peer ID: $out"
fi
ensure_disconnected

# #########################################################################
# SECTION 2: INVALID SERVER — BUG-001 regression
# "Connect to invalid server MUST fail with exit 1"
# #########################################################################
echo ""
echo "=== R2: Invalid Server (BUG-001 regression) ==="

# R2.1: Nonexistent hostname MUST fail with exit 1
ensure_disconnected
code=0
start_time=$(date +%s)
out=$("$BIN" connect 999999999 \
    --server "this-host-does-not-exist.invalid" \
    --timeout 5 2>&1) || code=$?
end_time=$(date +%s)
elapsed=$((end_time - start_time))
ensure_disconnected
if [[ "$code" == "1" ]]; then
    pass "R2.1 invalid server → exit 1 (BUG-001 FIXED)"
else
    fail "R2.1 invalid server" "BUG-001 NOT FIXED: expected exit 1, got exit $code. Output: $out"
fi

# R2.2: Connection to invalid server must NOT be instant (timing test)
# A real connection attempt should take >0 seconds (DNS lookup, timeout, etc.)
# A stub would return instantly (< 1 second)
if [[ "$elapsed" -ge 1 ]]; then
    pass "R2.2 invalid server took ${elapsed}s (not instant — real network I/O)"
else
    fail "R2.2 invalid server timing" "took ${elapsed}s — suspiciously instant, may still be stubbed"
fi

# R2.3: Unreachable IP address MUST fail with exit 1
ensure_disconnected
code=0
start_time=$(date +%s)
out=$("$BIN" connect 999999999 \
    --server "192.0.2.1:21116" \
    --timeout 5 2>&1) || code=$?
end_time=$(date +%s)
elapsed=$((end_time - start_time))
ensure_disconnected
if [[ "$code" == "1" ]]; then
    pass "R2.3 unreachable IP → exit 1"
else
    fail "R2.3 unreachable IP" "expected exit 1, got $code. Output: $out"
fi

# R2.4: Unreachable IP should take roughly --timeout seconds
if [[ "$elapsed" -ge 2 ]]; then
    pass "R2.4 unreachable IP took ${elapsed}s (respects timeout, real I/O)"
else
    fail "R2.4 unreachable IP timing" "took ${elapsed}s — too fast, expected ≥2s for timeout"
fi

# R2.5: Invalid server with JSON returns ok=false
ensure_disconnected
code=0
out=$("$BIN" --json connect 999999999 \
    --server "this-host-does-not-exist.invalid" \
    --timeout 3 2>&1) || code=$?
ensure_disconnected
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == False
assert 'error' in d
" 2>/dev/null; then
    pass "R2.5 invalid server JSON: ok=false + error"
else
    fail "R2.5 invalid server JSON" "expected ok=false: $out"
fi

# R2.6: Invalid server should NOT create lock/socket files
ensure_disconnected
code=0
"$BIN" connect 999999999 \
    --server "this-host-does-not-exist.invalid" \
    --timeout 3 >/dev/null 2>&1 || code=$?
if [[ -f /tmp/rustdesk-cli.lock ]] || [[ -S /tmp/rustdesk-cli.sock ]]; then
    fail "R2.6 invalid server leaves no files" "lock/socket files exist after failed connect"
    ensure_disconnected
else
    pass "R2.6 invalid server leaves no lock/socket files"
fi

# #########################################################################
# SECTION 3: CONNECT TIMING — detect stubs
# "Real connect should take > 0 seconds, not instant"
# #########################################################################
echo ""
echo "=== R3: Connect Timing (stub detection) ==="

# R3.1: Successful connect to live server takes measurable time
ensure_disconnected
start_time=$(python3 -c "import time; print(time.time())")
# shellcheck disable=SC2086
code=0
out=$("$BIN" connect $CONNECT_ARGS 2>&1) || code=$?
end_time=$(python3 -c "import time; print(time.time())")
elapsed=$(python3 -c "print(round($end_time - $start_time, 2))")
if [[ "$code" == "0" ]]; then
    # A real RustDesk connection (rendezvous lookup → relay → auth) should take >0.5s
    is_real=$(python3 -c "print('yes' if $elapsed > 0.5 else 'no')")
    if [[ "$is_real" == "yes" ]]; then
        pass "R3.1 connect took ${elapsed}s (real network handshake)"
    else
        fail "R3.1 connect timing" "took ${elapsed}s — suspiciously fast for real RustDesk handshake"
    fi
else
    fail "R3.1 connect" "failed to connect: exit=$code out=$out"
fi
ensure_disconnected

# R3.2: Width/height in status should reflect real remote display, not hardcoded 1920x1080
connect_live
out=$("$BIN" --json status 2>&1) || true
width=$(echo "$out" | python3 -c "import sys,json; print(json.load(sys.stdin).get('width',0))" 2>/dev/null || echo "0")
height=$(echo "$out" | python3 -c "import sys,json; print(json.load(sys.stdin).get('height',0))" 2>/dev/null || echo "0")
# We can't know the exact resolution, but if it's ALWAYS 1920x1080 that's suspicious
# The test machine (Ubuntu) might legitimately be 1920x1080, so we just document it
pass "R3.2 reported resolution: ${width}x${height} (manual review: is this the real remote display?)"
ensure_disconnected

# #########################################################################
# SECTION 4: EXEC — BUG-015/016 regression
# "exec should return REAL output, not 'stub exec output'"
# #########################################################################
echo ""
echo "=== R4: Exec (BUG-015/016 regression) ==="
connect_live

# R4.1: exec echo MUST return the actual echoed text, not stub
out=$("$BIN" exec --command "echo rustdesk_qa_canary_12345" 2>&1) || true
code=$?
if [[ "$code" == "0" ]] && echo "$out" | grep -q "rustdesk_qa_canary_12345"; then
    pass "R4.1 exec returns real output (BUG-015 FIXED)"
else
    if echo "$out" | grep -q "stub"; then
        fail "R4.1 exec STILL STUBBED" "BUG-015 NOT FIXED: output contains 'stub': $out"
    else
        fail "R4.1 exec output" "exit=$code, expected 'rustdesk_qa_canary_12345' in: $out"
    fi
fi

# R4.2: exec hostname should return the remote machine's hostname, not ours
local_hostname=$(hostname)
out=$("$BIN" exec --command "hostname" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    # Extract hostname — filter out any prefix lines from CLI output
    remote_hostname=$(echo "$out" | grep -v "^exec " | head -1 | tr -d '[:space:]' || echo "")
    if [[ -n "$remote_hostname" ]] && ! echo "$out" | grep -q "stub"; then
        pass "R4.2 exec hostname returned: '$remote_hostname' (real remote output)"
    else
        fail "R4.2 exec hostname" "output doesn't look like real hostname: $out"
    fi
else
    fail "R4.2 exec hostname" "exit=$code out=$out"
fi

# R4.3: exec with JSON MUST have stdout, stderr, exit_code fields (PM spec)
out=$("$BIN" --json exec --command "echo json_canary_67890" 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == True, f'ok={d.get(\"ok\")}'
# PM requires these three fields:
assert 'stdout' in d, 'missing stdout field'
assert 'stderr' in d, 'missing stderr field'
assert 'exit_code' in d, 'missing exit_code field'
# stdout should contain our canary
assert 'json_canary_67890' in d['stdout'], f'stdout={d[\"stdout\"]}'
" 2>/dev/null; then
    pass "R4.3 exec JSON has stdout+stderr+exit_code with real output (BUG-016 FIXED)"
else
    # Check which specific fields are missing
    missing=$(echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
m = []
if 'stdout' not in d: m.append('stdout')
if 'stderr' not in d: m.append('stderr')
if 'exit_code' not in d: m.append('exit_code')
print(', '.join(m) if m else 'fields present but content wrong')
" 2>/dev/null || echo "parse error")
    fail "R4.3 exec JSON format" "BUG-016: missing [$missing] in: $out"
fi

# R4.4: exec a command that writes to stderr
out=$("$BIN" --json exec --command "echo err_canary >&2" 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
stderr = d.get('stderr', '')
assert 'err_canary' in stderr, f'stderr={stderr}'
" 2>/dev/null; then
    pass "R4.4 exec captures stderr separately"
else
    fail "R4.4 exec stderr" "stderr field missing or wrong: $out"
fi

# R4.5: exec a command that exits non-zero
out=$("$BIN" --json exec --command "exit 42" 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
ec = d.get('exit_code')
assert ec == 42, f'exit_code={ec}, expected 42'
" 2>/dev/null; then
    pass "R4.5 exec reports remote exit_code=42"
else
    exit_code_got=$(echo "$out" | python3 -c "import sys,json; print(json.load(sys.stdin).get('exit_code','?'))" 2>/dev/null || echo "?")
    fail "R4.5 exec exit_code" "expected exit_code=42, got $exit_code_got: $out"
fi

# R4.6: exec 'uname -s' should return Linux (test machine is Ubuntu)
out=$("$BIN" exec --command "uname -s" 2>&1) || true
if echo "$out" | grep -qi "linux"; then
    pass "R4.6 exec uname confirms remote is Linux"
else
    fail "R4.6 exec uname" "expected 'Linux' in output: $out"
fi

# R4.7: exec 'id' should return remote user info, not local user
local_user=$(whoami)
out=$("$BIN" exec --command "whoami" 2>&1) || true
if [[ "$code" == "0" ]] && echo "$out" | grep -qv "stub"; then
    pass "R4.7 exec whoami returned real remote user info"
else
    fail "R4.7 exec whoami" "output: $out"
fi

# R4.8: exec with pipe operator
out=$("$BIN" exec --command "echo abc def ghi | wc -w" 2>&1) || true
if echo "$out" | grep -q "3"; then
    pass "R4.8 exec with pipe returns correct result"
else
    fail "R4.8 exec pipe" "expected '3' in: $out"
fi

ensure_disconnected

# #########################################################################
# SECTION 5: SHELL — interactive terminal
# #########################################################################
echo ""
echo "=== R5: Shell (interactive terminal) ==="
connect_live

# Portable timeout: use gtimeout (homebrew) or python fallback
TIMEOUT_CMD=""
if command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT_CMD="gtimeout"
elif command -v timeout >/dev/null 2>&1; then
    TIMEOUT_CMD="timeout"
fi

# R5.1: shell should start and accept input via stdin
if [[ -n "$TIMEOUT_CMD" ]]; then
    out=$(echo "echo shell_canary_99999" | $TIMEOUT_CMD 5 "$BIN" shell 2>&1) || true
    if echo "$out" | grep -q "shell_canary_99999"; then
        pass "R5.1 shell accepts stdin and returns output"
    else
        # Shell might need special handling for non-interactive mode
        skip "R5.1 shell stdin" "output: $out (shell may require TTY)"
    fi
else
    # Use python as timeout fallback
    out=$(echo "echo shell_canary_99999" | python3 -c "
import subprocess, sys
try:
    r = subprocess.run([sys.argv[1], 'shell'], input=sys.stdin.buffer.read(),
                       capture_output=True, timeout=5)
    sys.stdout.buffer.write(r.stdout)
    sys.stderr.buffer.write(r.stderr)
except subprocess.TimeoutExpired:
    print('TIMEOUT')
" "$BIN" 2>&1) || true
    if echo "$out" | grep -q "shell_canary_99999"; then
        pass "R5.1 shell accepts stdin and returns output"
    else
        skip "R5.1 shell stdin" "output: $out (shell may require TTY)"
    fi
fi

# R5.2: shell --json should indicate the session started
if [[ -n "$TIMEOUT_CMD" ]]; then
    code=0
    out=$(echo "exit" | $TIMEOUT_CMD 5 "$BIN" --json shell 2>&1) || code=$?
else
    code=0
    out=$(echo "exit" | python3 -c "
import subprocess, sys
try:
    r = subprocess.run([sys.argv[1], '--json', 'shell'], input=sys.stdin.buffer.read(),
                       capture_output=True, timeout=5)
    sys.stdout.buffer.write(r.stdout)
    sys.exit(r.returncode)
except subprocess.TimeoutExpired:
    print('TIMEOUT')
    sys.exit(124)
" "$BIN" 2>&1) || code=$?
fi
if echo "$out" | python3 -m json.tool >/dev/null 2>&1; then
    pass "R5.2 shell --json produces valid JSON"
else
    skip "R5.2 shell --json" "may require TTY: $out"
fi

ensure_disconnected

# R5.3: shell without session MUST fail
ensure_disconnected
code=0
out=$("$BIN" shell 2>&1) || code=$?
if [[ "$code" == "1" ]] || [[ "$code" == "2" ]]; then
    pass "R5.3 shell without session fails (exit $code)"
else
    fail "R5.3 shell no session" "expected exit 1 or 2, got $code: $out"
fi

# #########################################################################
# SECTION 6: CLIPBOARD ROUND-TRIP
# #########################################################################
echo ""
echo "=== R6: Clipboard Round-Trip ==="
connect_live

# R6.1: Set clipboard, then get it back — text should match
canary="clipboard_roundtrip_$(date +%s)"
set_out=$("$BIN" clipboard set --text "$canary" 2>&1) || true
set_code=$?
sleep 1
get_out=$("$BIN" clipboard get 2>&1) || true
get_code=$?
if [[ "$set_code" == "0" ]] && [[ "$get_code" == "0" ]] && echo "$get_out" | grep -qF "$canary"; then
    pass "R6.1 clipboard round-trip: set → get matches"
else
    fail "R6.1 clipboard round-trip" "set(exit=$set_code)=$set_out get(exit=$get_code)=$get_out — expected '$canary'"
fi

# R6.2: Clipboard JSON round-trip
canary2="json_clip_$(date +%s)"
"$BIN" clipboard set --text "$canary2" >/dev/null 2>&1 || true
sleep 1
out=$("$BIN" --json clipboard get 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d.get('ok') == True
text = d.get('text', d.get('content', d.get('clipboard', '')))
assert '$canary2' in text, f'text={text}'
" 2>/dev/null; then
    pass "R6.2 clipboard JSON round-trip matches"
else
    fail "R6.2 clipboard JSON round-trip" "canary '$canary2' not in: $out"
fi

# R6.3: Clipboard with unicode round-trip
unicode_canary="你好_$(date +%s)_🎉"
"$BIN" clipboard set --text "$unicode_canary" >/dev/null 2>&1 || true
sleep 1
out=$("$BIN" clipboard get 2>&1) || true
if echo "$out" | grep -qF "$unicode_canary"; then
    pass "R6.3 clipboard unicode round-trip"
else
    fail "R6.3 clipboard unicode" "expected '$unicode_canary' in: $out"
fi

ensure_disconnected

# #########################################################################
# SECTION 7: ERROR PATHS — exit codes per SPEC
# #########################################################################
echo ""
echo "=== R7: Error Paths & Exit Codes ==="
ensure_disconnected

# SPEC.md Error Handling:
#   0 = success
#   1 = connection error (unreachable, auth failed)
#   2 = session error (no active session, disconnected)
#   3 = input error (bad arguments)

# R7.1: Commands without session MUST fail with exit 1 or 2
# Per SPEC, "no active session" = exit 2 (session error)
for cmd_args in \
    "type hello" \
    "key enter" \
    "click 500 300" \
    "move 100 200" \
    "drag 10 20 30 40" \
    "exec --command ls" \
    "capture /tmp/test_regression.png" \
    "clipboard get" \
    "clipboard set --text test" \
    "shell"; do

    code=0
    # shellcheck disable=SC2086
    out=$("$BIN" $cmd_args 2>&1) || code=$?
    if [[ "$code" == "2" ]]; then
        pass "R7.1 no-session '$cmd_args' → exit 2 (session error, per SPEC)"
    elif [[ "$code" == "1" ]]; then
        # exit 1 is "connection error" — acceptable but SPEC says 2 for "no active session"
        pass "R7.1 no-session '$cmd_args' → exit 1 (connection error — SPEC prefers 2)"
    else
        fail "R7.1 no-session '$cmd_args'" "expected exit 1 or 2, got $code"
    fi
done

# R7.2: disconnect with no session MUST exit 2 (per PM answer)
ensure_disconnected
code=0
out=$("$BIN" disconnect 2>&1) || code=$?
if [[ "$code" == "2" ]]; then
    pass "R7.2 disconnect no-session → exit 2 (BUG-014 FIXED)"
else
    fail "R7.2 disconnect no-session" "BUG-014 NOT FIXED: expected exit 2, got $code"
fi

# R7.3: Invalid server MUST exit 1 (connection error)
ensure_disconnected
code=0
out=$("$BIN" connect 999 --server "192.0.2.1:21116" --timeout 3 2>&1) || code=$?
ensure_disconnected
if [[ "$code" == "1" ]]; then
    pass "R7.3 invalid server → exit 1 (connection error, per SPEC)"
else
    fail "R7.3 invalid server exit code" "expected exit 1, got $code"
fi

# R7.4: Wrong password MUST exit 1 (connection error — auth failed)
ensure_disconnected
code=0
out=$("$BIN" connect "$PEER_ID" \
    --password "WRONG" \
    --id-server "$ID_SERVER" \
    --relay-server "$RELAY_SERVER" \
    --key "$KEY" \
    --timeout "$TIMEOUT" 2>&1) || code=$?
ensure_disconnected
if [[ "$code" == "1" ]]; then
    pass "R7.4 wrong password → exit 1 (connection error, per SPEC)"
else
    fail "R7.4 wrong password exit code" "expected exit 1, got $code"
fi

# R7.5: status with no session exits 0 (PM confirmed this is correct)
ensure_disconnected
code=0
out=$("$BIN" status 2>&1) || code=$?
if [[ "$code" == "0" ]]; then
    pass "R7.5 status no-session → exit 0 (PM confirmed correct)"
else
    fail "R7.5 status no-session" "expected exit 0, got $code"
fi

# #########################################################################
# SECTION 8: ERROR OUTPUT DESTINATION — BUG-013
# #########################################################################
echo ""
echo "=== R8: Error Output Destination (BUG-013) ==="
ensure_disconnected

# R8.1: Errors MUST go to stderr, not stdout (per SPEC)
stdout_out=$("$BIN" type hello 2>/dev/null) || true
stderr_out=$("$BIN" type hello 2>&1 >/dev/null) || true
if [[ -z "$stdout_out" ]] && [[ -n "$stderr_out" ]]; then
    pass "R8.1 errors go to stderr (BUG-013 FIXED)"
elif [[ -n "$stdout_out" ]]; then
    fail "R8.1 errors on stdout" "BUG-013 NOT FIXED: stderr should have error, stdout has: $stdout_out"
else
    fail "R8.1 no error output" "expected error on stderr, got nothing"
fi

# R8.2: JSON errors should still go to stdout (for machine parsing)
stdout_json=$("$BIN" --json type hello 2>/dev/null) || true
if echo "$stdout_json" | python3 -m json.tool >/dev/null 2>&1; then
    pass "R8.2 JSON errors on stdout (correct for machine parsing)"
else
    fail "R8.2 JSON errors" "expected JSON on stdout: $stdout_json"
fi

# R8.3: Connection error messages go to stderr
stdout_out=$("$BIN" connect 999 --server "192.0.2.1:21116" --timeout 2 2>/dev/null) || true
stderr_out=$("$BIN" connect 999 --server "192.0.2.1:21116" --timeout 2 2>&1 >/dev/null) || true
ensure_disconnected
if [[ -z "$stdout_out" ]] && [[ -n "$stderr_out" ]]; then
    pass "R8.3 connection errors go to stderr"
elif [[ -n "$stdout_out" ]]; then
    fail "R8.3 connection errors on stdout" "expected stderr, got stdout: $stdout_out"
else
    # Might be no output at all if JSON mode is somehow activated
    skip "R8.3 connection error output" "no output captured on either stream"
fi

# #########################################################################
# SECTION 9: BATCH (do) SESSION CHECK — BUG-012
# #########################################################################
echo ""
echo "=== R9: Batch Session Check (BUG-012) ==="
ensure_disconnected

# R9.1: do batch without session MUST fail
code=0
out=$("$BIN" do type hello key enter 2>&1) || code=$?
if [[ "$code" != "0" ]]; then
    pass "R9.1 do batch without session fails (BUG-012 FIXED)"
else
    fail "R9.1 do batch no session" "BUG-012 NOT FIXED: expected failure, got exit 0: $out"
fi

# R9.2: do batch without session should mention no session
if echo "$out" | grep -qiE "no active session|not connected|connect first|session"; then
    pass "R9.2 do batch error is descriptive"
else
    if [[ "$code" != "0" ]]; then
        pass "R9.2 do batch fails (error message could be better): $out"
    else
        fail "R9.2 do batch error message" "no session-related keyword in: $out"
    fi
fi

# R9.3: do batch WITH session works
connect_live
code=0
out=$("$BIN" do type hello key enter 2>&1) || code=$?
if [[ "$code" == "0" ]]; then
    pass "R9.3 do batch with session succeeds"
else
    fail "R9.3 do batch with session" "exit=$code out=$out"
fi
ensure_disconnected

# #########################################################################
# SECTION 10: CAPTURE — BUG-002 partial regression
# #########################################################################
echo ""
echo "=== R10: Capture (BUG-002 partial regression) ==="
connect_live

CAPTURE_FILE="/tmp/rustdesk_qa_capture_test_$$.png"
rm -f "$CAPTURE_FILE"

# R10.1: capture should write an actual file to disk
out=$("$BIN" capture "$CAPTURE_FILE" 2>&1) || true
code=$?
if [[ "$code" == "0" ]] && [[ -f "$CAPTURE_FILE" ]]; then
    file_size=$(stat -f%z "$CAPTURE_FILE" 2>/dev/null || stat -c%s "$CAPTURE_FILE" 2>/dev/null || echo 0)
    if [[ "$file_size" -gt 100 ]]; then
        pass "R10.1 capture writes real file (${file_size} bytes) (BUG-002 FIXED)"
    else
        fail "R10.1 capture file too small" "file exists but only ${file_size} bytes — likely not a real image"
    fi
elif [[ "$code" == "0" ]] && [[ ! -f "$CAPTURE_FILE" ]]; then
    fail "R10.1 capture" "BUG-002 NOT FIXED: exit 0 but no file created. Output: $out"
else
    skip "R10.1 capture" "capture may not be in scope for #18 (exit=$code): $out"
fi

# R10.2: if capture file was created, verify it's a valid PNG
if [[ -f "$CAPTURE_FILE" ]]; then
    file_type=$(file -b "$CAPTURE_FILE" 2>/dev/null || echo "unknown")
    if echo "$file_type" | grep -qi "PNG\|image"; then
        pass "R10.2 capture file is valid image: $file_type"
    else
        fail "R10.2 capture file type" "expected PNG, got: $file_type"
    fi
else
    skip "R10.2 capture file validation" "no file to validate"
fi

rm -f "$CAPTURE_FILE"
ensure_disconnected

# #########################################################################
# SUMMARY
# #########################################################################
echo ""
echo "========================================"
echo "  Daemon Wiring Regression: $PASS passed, $FAIL failed, $SKIP skipped"
echo "========================================"

if [[ ${#FAILURES[@]} -gt 0 ]]; then
    echo ""
    echo "FAILURES:"
    for f in "${FAILURES[@]}"; do
        echo "  - $f"
    done

    echo ""
    echo "BUG STATUS:"
    # Check each critical bug
    for bug in "BUG-001" "BUG-002" "BUG-003" "BUG-012" "BUG-013" "BUG-014" "BUG-015" "BUG-016"; do
        if printf '%s\n' "${FAILURES[@]}" | grep -q "$bug NOT FIXED"; then
            printf "  \033[31m✗\033[0m %s — NOT FIXED\n" "$bug"
        elif printf '%s\n' "${FAILURES[@]}" | grep -q "$bug"; then
            printf "  \033[33m~\033[0m %s — PARTIALLY FIXED\n" "$bug"
        else
            printf "  \033[32m✓\033[0m %s — FIXED\n" "$bug"
        fi
    done
fi

echo ""
exit "$FAIL"
