# Findings & Decisions

## Deployment Inputs
- Peer ID: `308235080`
- Password: `Evas@2026`
- ID server: `115.238.185.55:50076`
- Relay server: `115.238.185.55:50077`
- Server key: `SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=`

## Deployment Payload
- Target file: `/home/evas/rmsnorm_op.py`
- Expected verification output: `SUCCESS: RMSNorm verification passed.`

## Planned Execution Order
1. `cargo run -- connect ...`
2. `cargo run -- exec --command "cat > /home/evas/rmsnorm_op.py ..."`
3. `cargo run -- exec --command "python3 /home/evas/rmsnorm_op.py"`
4. Fallback to terminal piping only if daemon exec fails

## Actual Execution Findings
- `cargo run -- connect ...` reported success, but the detached daemon died immediately and left only a stale lock/socket.
- Running `target/debug/rustdesk-cli --daemon ...` in the foreground kept the session alive and allowed `exec` to work reliably enough for deployment.
- The script file was created at `/home/evas/rmsnorm_op.py`.
- Initial verification failed because the remote host lacked `numpy`.
- Remote Python environment details:
  - Ubuntu 22.04.5 LTS
  - user `evas`
  - no `pip`
  - no `ensurepip`
- User-space recovery worked:
  - bootstrapped `pip` via `curl https://bootstrap.pypa.io/get-pip.py`
  - installed `numpy` into `~/.local`
- Final verification output from the remote host:
  - `SUCCESS: RMSNorm verification passed.`
