# rustdesk-cli Dogfood Guide

This guide provides instructions for using and testing the current capabilities of `rustdesk-cli`.

## 1. Connection Setup

To connect to a remote peer, you need the Peer ID, password, and server configuration.

### Server Configuration
For Evas testing, use the following server settings:
- **ID Server:** `115.238.185.55:50076`
- **Relay Server:** `115.238.185.55:50077`
- **Server Key:** `SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=`

### Basic Connection Command
```bash
rustdesk-cli connect 308235080 \
  --password "Evas@2026" \
  --id-server "115.238.185.55:50076" \
  --relay-server "115.238.185.55:50077" \
  --key "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc="
```

## 2. Daemon Mode: Connect + Exec Workflow

By default, the `connect` command spawns a background daemon. This daemon maintains a persistent connection to the peer, allowing for fast subsequent commands.

### Workflow:
1. **Connect:** Establish the persistent session.
   ```bash
   rustdesk-cli connect 308235080 --password "Evas@2026" --server "115.238.185.55" --key "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc="
   ```
2. **Execute Commands:** Use `exec` to run non-interactive commands.
   ```bash
   rustdesk-cli exec --command "ls -la"
   rustdesk-cli exec --command "whoami"
   ```
3. **Check Status:**
   ```bash
   rustdesk-cli status
   ```
4. **Disconnect:** Terminate the daemon and the session.
   ```bash
   rustdesk-cli disconnect
   ```

## 3. Direct --terminal Mode: Interactive Shell

If you need a real-time interactive terminal (e.g., for manual troubleshooting or using `vim`), use the `--terminal` flag with `connect`.

```bash
rustdesk-cli connect 308235080 --terminal --password "Evas@2026" --server "115.238.185.55" --key "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc="
```
*Note: In this mode, `rustdesk-cli` acts as a terminal emulator. `Ctrl+C` will be sent to the remote shell.*

## 4. Known Limitations

- **Connection Setup Time:** Establishing a secure connection (Rendezvous -> Relay -> NaCl Handshake -> Login) typically takes **7-8 seconds**.
- **Exec Timeout:** Remote commands executed via `exec` have a default timeout of **30 seconds**.
- **Interactive Apps in Exec:** `exec` is for non-interactive commands. Do not run commands that require user input (like `sudo` without `-S` or `apt` without `-y`) via `exec`. Use `--terminal` instead.
- **State Persistence:** Each `exec` command runs in a fresh shell environment on the remote side. Environment variables or `cd` commands from a previous `exec` will not persist.

## 5. Example Development Workflow

### Edit a file remotely
For small edits, use a one-liner or redirected `cat`:
```bash
rustdesk-cli exec --command "echo 'fn main() { println!(\"Hello Evas\"); }' > hello.rs"
```
For complex edits, open an interactive shell:
```bash
rustdesk-cli connect 308235080 --terminal ...
# Inside the terminal:
vim hello.rs
```

### Compile and Run
```bash
# Compile
rustdesk-cli exec --command "rustc hello.rs"

# Run
rustdesk-cli exec --command "./hello"
```

### Synchronize Clipboard
```bash
# Set remote clipboard from local text
rustdesk-cli clipboard set --text "Shared Secret"

# Get remote clipboard content
rustdesk-cli clipboard get
```
