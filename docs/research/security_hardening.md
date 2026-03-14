# Security Hardening Research

This document outlines the security posture of `rustdesk-cli`, identifies potential risks, and provides recommendations for hardening in production environments.

## 1. Credential Management

Currently, `rustdesk-cli` provides three methods for handling peer passwords:

| Method | Flag/Env | Risk Profile | Recommendation |
| :--- | :--- | :--- | :--- |
| **CLI Argument** | `--password` | **HIGH**. Visible in `ps aux` and shell history. | Avoid in production/multi-user systems. |
| **Env Variable** | `RUSTDESK_PASSWORD` | **MEDIUM**. Visible to other processes by same user. | Better for CI/CD or containerized agents. |
| **Stdin Pipe** | `--password-stdin` | **LOW**. Most secure for automated scripts. | **Primary choice** for high-security automation. |

### Future Hardening:
- Implement OS Keychain/SecretService integration for long-term storage of frequently used peer keys.
- Support a `.env` or config file with `0600` permissions.

## 2. Encryption and Handshake

The RustDesk protocol uses a robust two-layer encryption strategy.

### Phase 4 Handshake:
1. **Key Exchange:** The client generates a random **32-byte symmetric session key** for *every* new connection.
2. **Protection:** This session key is sealed using **NaCl `crypto_box`** (`Salsa20` + `Poly1305`) against the peer's Curve25519 public key.
3. **Nonce:** A zeroed nonce is used for the `crypto_box`, which is safe because the ephemeral session key pair is never reused.

### Session Encryption:
- **Cipher:** **XSalsa20-Poly1305** (`secretbox`).
- **PFS:** Because session keys are generated randomly per-connection, the protocol provides **Perfect Forward Secrecy (PFS)**.
- **Nonces:** Uses a monotonic 64-bit sequence counter (Send/Recv) to prevent replay attacks and ensure unique nonces for every frame.

## 3. Sandbox Escape and Injection Risks

### Exec Sentinel Exploitation
The `rustdesk-cli exec` command uses a unique sentinel (e.g., `__RDCLI_...__`) to detect command completion.
- **Risk:** A malicious peer could craft a script that prints the sentinel string to stdout early, causing the local CLI to truncate output and incorrectly parse the exit code.
- **Mitigation:** The sentinel is generated using 128 bits of entropy (nanosecond timestamp + random hex) for each execution, making it statistically impossible to guess. We should also ensure the sentinel is matched only at the *start* of a new line.

### Path Traversal in File Transfer
- **Risk:** A peer could send a `FileResponse::Digest` with a relative path like `../../etc/passwd`.
- **Audit:** Our current `PushTransfer` implementation calculates the destination path locally and validates it. However, any future "Pull" or "Receive" implementation must strictly sanitize paths to prevent writing outside the target directory.

## 4. Audit of `--dangerously-skip-permissions`

This flag is intended for AI agents and automated nodes.

**What it bypasses:**
- Skips the local `PermissionManager` check (defined in `permissions.rs`).
- On the *controlled* side (server), it bypasses the IPC call to the Connection Manager UI, auto-authorizing incoming requests.

**Guardrails Recommended:**
- Even in "skip" mode, the daemon should only accept connections from a **whitelist of Peer IDs**.
- Log all skip-mode connections to a persistent audit trail.

## 5. Production Deployment Checklist

1. [ ] **Never** pass passwords via CLI flags; use `stdin` or env vars.
2. [ ] **Always** provide the `--key` (Server Public Key) to prevent Man-in-the-Middle (MITM) attacks during rendezvous.
3. [ ] Run the daemon process under a **non-root** dedicated user.
4. [ ] Restrict the UDS socket (`/tmp/rustdesk-cli.sock`) to `0600` permissions (already implemented).
5. [ ] If using `--sandbox`, ensure the `rustdesk-cli.toml` is read-only for the application user.
6. [ ] For long-running agents, monitor the daemon's heartbeat logs for unauthorized reconnection attempts.
