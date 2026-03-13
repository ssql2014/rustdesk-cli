#!/usr/bin/env bash
# =============================================================================
# rustdesk-cli QA — Live Server Integration Tests
# Requires the test server from TEST_CONFIG.md to be running.
# Run from the repo root:  bash tests/qa/live_server_test.sh
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

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "=== Building binary ==="
cargo build 2>&1
echo ""

ensure_disconnected

# =============================================================================
echo "=== Live T1: Connect / Status / Disconnect Lifecycle ==="
# =============================================================================

# LT1.1: Connect to test server
# shellcheck disable=SC2086
out=$("$BIN" connect $CONNECT_ARGS 2>&1) || true
code=$?
if [[ "$code" == "0" ]] && echo "$out" | grep -q "connected"; then
    pass "LT1.1 connect to test server succeeds"
else
    fail "LT1.1 connect" "exit=$code out=$out"
    echo "FATAL: Cannot connect to test server. Aborting live tests."
    exit 1
fi

# LT1.2: Lock file exists after connect
if [[ -f /tmp/rustdesk-cli.lock ]]; then
    pass "LT1.2 lock file created"
else
    fail "LT1.2 lock file" "missing after connect"
fi

# LT1.3: Socket exists after connect
if [[ -S /tmp/rustdesk-cli.sock ]]; then
    pass "LT1.3 socket created"
else
    fail "LT1.3 socket" "missing after connect"
fi

# LT1.4: Lock file has correct permissions (0600)
perms=$(stat -f '%Lp' /tmp/rustdesk-cli.lock 2>/dev/null || stat -c '%a' /tmp/rustdesk-cli.lock 2>/dev/null)
if [[ "$perms" == "600" ]]; then
    pass "LT1.4 lock file permissions are 0600"
else
    fail "LT1.4 lock file permissions" "expected 600, got $perms"
fi

# LT1.5: Socket file has correct permissions (0600)
perms=$(stat -f '%Lp' /tmp/rustdesk-cli.sock 2>/dev/null || stat -c '%a' /tmp/rustdesk-cli.sock 2>/dev/null)
if [[ "$perms" == "600" ]]; then
    pass "LT1.5 socket permissions are 0600"
else
    fail "LT1.5 socket permissions" "expected 600, got $perms"
fi

# LT1.6: Status shows connected with peer ID
out=$("$BIN" status 2>&1)
if echo "$out" | grep -q "$PEER_ID"; then
    pass "LT1.6 status shows peer ID"
else
    fail "LT1.6 status peer ID" "expected $PEER_ID in: $out"
fi

# LT1.7: Status shows resolution
if echo "$out" | grep -qE "[0-9]+.*[0-9]+"; then
    pass "LT1.7 status shows resolution"
else
    fail "LT1.7 status resolution" "no resolution in: $out"
fi

# LT1.8: JSON status has all expected fields
out=$("$BIN" --json status 2>&1)
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d['ok'] == True
assert d['connected'] == True
assert 'id' in d
assert 'width' in d
assert 'height' in d
" 2>/dev/null; then
    pass "LT1.8 JSON status has all fields"
else
    fail "LT1.8 JSON status fields" "missing expected fields: $out"
fi

# LT1.9: Double connect fails gracefully
# shellcheck disable=SC2086
code=0
out=$("$BIN" connect $CONNECT_ARGS 2>&1) || code=$?
if [[ "$code" != "0" ]]; then
    pass "LT1.9 double connect rejected (exit $code)"
else
    if echo "$out" | grep -qi "already\|daemon"; then
        fail "LT1.9 double connect" "error message present but exit 0: $out"
    else
        fail "LT1.9 double connect" "should fail but got exit 0: $out"
    fi
fi

# LT1.10: Disconnect
out=$("$BIN" disconnect 2>&1)
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT1.10 disconnect succeeds"
else
    fail "LT1.10 disconnect" "exit=$code"
fi
sleep 0.5

# LT1.11: Lock file removed after disconnect
if [[ ! -f /tmp/rustdesk-cli.lock ]]; then
    pass "LT1.11 lock file removed"
else
    fail "LT1.11 lock file" "still exists after disconnect"
fi

# LT1.12: Socket removed after disconnect
if [[ ! -S /tmp/rustdesk-cli.sock ]]; then
    pass "LT1.12 socket removed"
else
    fail "LT1.12 socket" "still exists after disconnect"
fi

# LT1.13: Status shows disconnected after disconnect
out=$("$BIN" status 2>&1)
if echo "$out" | grep -qi "disconnected\|connected.*false"; then
    pass "LT1.13 status shows disconnected"
else
    fail "LT1.13 status after disconnect" "unexpected: $out"
fi

# =============================================================================
echo ""
echo "=== Live T2: Connect with JSON ==="
# =============================================================================
ensure_disconnected

# LT2.1: JSON connect output
# shellcheck disable=SC2086
out=$("$BIN" --json connect $CONNECT_ARGS 2>&1) || true
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d['ok'] == True
assert d['command'] == 'connect'
" 2>/dev/null; then
    pass "LT2.1 JSON connect output valid"
else
    fail "LT2.1 JSON connect" "invalid: $out"
fi

# LT2.2: JSON disconnect output
out=$("$BIN" --json disconnect 2>&1)
if echo "$out" | python3 -c "
import sys, json
d = json.load(sys.stdin)
assert d['ok'] == True
assert d['command'] == 'disconnect'
" 2>/dev/null; then
    pass "LT2.2 JSON disconnect output valid"
else
    fail "LT2.2 JSON disconnect" "invalid: $out"
fi

# =============================================================================
echo ""
echo "=== Live T3: Connect via RUSTDESK_PASSWORD env var ==="
# =============================================================================
ensure_disconnected

# LT3.1: Connect using env var instead of --password
code=0
out=$(RUSTDESK_PASSWORD="$PASSWORD" "$BIN" connect "$PEER_ID" \
    --id-server "$ID_SERVER" \
    --relay-server "$RELAY_SERVER" \
    --key "$KEY" \
    --timeout "$TIMEOUT" 2>&1) || code=$?
if [[ "$code" == "0" ]] && echo "$out" | grep -q "connected"; then
    pass "LT3.1 connect via RUSTDESK_PASSWORD env var"
else
    # Might fail because env var not implemented yet — that's a finding
    fail "LT3.1 RUSTDESK_PASSWORD env var" "exit=$code out=$out"
fi
ensure_disconnected

# =============================================================================
echo ""
echo "=== Live T4: Piped Workflow (connect → commands → disconnect) ==="
# =============================================================================
ensure_disconnected

# Connect first
# shellcheck disable=SC2086
"$BIN" connect $CONNECT_ARGS >/dev/null 2>&1 || true
sleep 1

# LT4.1: type command
out=$("$BIN" type "hello qa test" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.1 type command succeeds"
else
    fail "LT4.1 type command" "exit=$code out=$out"
fi

# LT4.2: key command
out=$("$BIN" key enter 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.2 key enter succeeds"
else
    fail "LT4.2 key enter" "exit=$code out=$out"
fi

# LT4.3: key with modifiers
out=$("$BIN" key a --modifiers ctrl 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.3 key with modifier succeeds"
else
    fail "LT4.3 key with modifier" "exit=$code out=$out"
fi

# LT4.4: click command
out=$("$BIN" click 500 300 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.4 click succeeds"
else
    fail "LT4.4 click" "exit=$code out=$out"
fi

# LT4.5: click with button
out=$("$BIN" click 500 300 --button right 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.5 click right button succeeds"
else
    fail "LT4.5 click right" "exit=$code out=$out"
fi

# LT4.6: move command
out=$("$BIN" move 100 200 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.6 move succeeds"
else
    fail "LT4.6 move" "exit=$code out=$out"
fi

# LT4.7: drag command
out=$("$BIN" drag 100 200 300 400 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.7 drag succeeds"
else
    fail "LT4.7 drag" "exit=$code out=$out"
fi

# LT4.8: batch (do) command
out=$("$BIN" do type world key enter 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.8 batch (do) succeeds"
else
    fail "LT4.8 batch (do)" "exit=$code out=$out"
fi

# LT4.9: exec command
out=$("$BIN" exec --command "echo hello" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.9 exec succeeds"
else
    fail "LT4.9 exec" "exit=$code out=$out"
fi

# LT4.10: clipboard set
out=$("$BIN" clipboard set --text "qa clipboard test" 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.10 clipboard set succeeds"
else
    fail "LT4.10 clipboard set" "exit=$code out=$out"
fi

# LT4.11: clipboard get
out=$("$BIN" clipboard get 2>&1) || true
code=$?
if [[ "$code" == "0" ]]; then
    pass "LT4.11 clipboard get succeeds"
else
    fail "LT4.11 clipboard get" "exit=$code out=$out"
fi

ensure_disconnected

# =============================================================================
# Summary
# =============================================================================
echo ""
echo "========================================"
echo "  Live QA Results: $PASS passed, $FAIL failed, $SKIP skipped"
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
