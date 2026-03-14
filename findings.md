# Findings & Decisions

## Root Cause
- The background daemon spawned from `cargo run -- connect ...` was not fully detached from the parent process/session.
- The detached startup path also signaled readiness too early by writing `/tmp/rustdesk-cli.lock` before `initialize_stream_for_mode()` completed.

## Why Foreground `--daemon` Worked
- Running `target/debug/rustdesk-cli --daemon ...` directly bypassed the parent/child handoff.
- That confirmed the connection/auth/relay logic itself was not the primary problem.

## Code Changes
- In [`src/daemon.rs`](/Users/qlss/Documents/Projects/rustdesk-cli/src/daemon.rs):
  - added Unix `pre_exec` + `libc::setsid()` in `spawn_daemon()` so the background daemon starts in its own session
  - moved `LockFile::write(SOCKET_PATH)` to after `initialize_stream_for_mode()`
  - now writes a startup error if post-auth stream initialization fails before readiness is published

## Live Verification
- `cargo run -- connect 308235080 --password 'Evas@2026' --id-server 115.238.185.55:50076 --relay-server 115.238.185.55:50077 --key 'SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc='`
  returned `connected id=308235080 width=1920 height=1080`
- After a 3-second delay, `cargo run -- status` returned:
  - `connected id=308235080 width=1920 height=1080`

## Test Outcome
- `cargo test` passed once rerun without an active live daemon left over from the handoff check

