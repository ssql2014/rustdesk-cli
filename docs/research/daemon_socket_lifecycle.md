# Research: Daemon Socket Lifecycle and macOS Edge Cases

This document investigates the behavior of Unix Domain Sockets (UDS) on macOS, specifically addressing why stale socket files can prevent daemon restarts and how to handle these cases robustly.

## 1. macOS-Specific UDS Behavior

Unlike Linux, macOS (and other BSD-derived systems) has several unique constraints and security policies affecting Unix Domain Sockets.

### Filesystem-Only Namespace
- **No Abstract Namespace:** macOS **does not support** the Linux-specific "abstract namespace" (sockets starting with a `\0` byte). 
- **Persistence:** All UDS on macOS are "pathname" sockets. They are represented by a physical file on disk. If the process terminates without calling `unlink()`, the file **persists indefinitely**.
- **The `EADDRINUSE` Error:** When a daemon tries to `bind()` to a path where a file already exists, it fails with `EADDRINUSE`, even if no process is actually listening on that socket.

### Security and Path Selection (`/tmp` vs `$TMPDIR`)
- **Global `/tmp`:** While `/tmp` is convenient and has short paths, it is a global directory. Placing sockets here is a security risk (socket squatting) and is **strictly blocked** for sandboxed applications.
- **`$TMPDIR` (DARWIN_USER_TEMP_DIR):** macOS provides a user-specific, randomized temporary directory via the `$TMPDIR` environment variable.
    - **Pros:** Inherently isolated per user; allowed for sandboxed and hardened applications.
    - **Cons:** Paths are often very long (e.g., `/var/folders/xx/.../T/`), which can conflict with the 104-character limit.
- **The 104-Character Limit:** macOS restricts the `sun_path` in `sockaddr_un` to **104 bytes** (Linux is 108). If the path to `$TMPDIR` is too long, `bind()` will fail with `ENAMETOOLONG`.

## 2. Process Death and Stale Artifacts

A socket file can persist without a matching lock file under several conditions:
- **SIGKILL (Killed: 9):** The process is terminated immediately by the kernel. Destructors (like `Drop` in Rust) are not run, and the `unlink()` call is skipped.
- **OOM Kill / Crash:** Similar to `SIGKILL`, the process exits abruptly without cleanup.
- **Power Loss:** The filesystem is not cleaned up on reboot (though `/tmp` is cleared on many systems, some persistent `/var` paths are not).
- **Race Conditions:** If the daemon creates the socket file but crashes *before* creating the lock file, or if the lock file is removed manually while the daemon is still running (unlikely but possible).

## 3. Best Practices for Robust Lifecycle Management

### The "Connect-then-Unlink" Pattern
This is the industry standard for handling stale pathname sockets without using abstract namespaces.

**Logic for `bind()`:**
1. Try to `connect()` to the socket path.
2. If `connect()` **succeeds**: Another instance is alive. **Exit with error.**
3. If `connect()` **fails with `ECONNREFUSED`**: The file exists but no one is listening (it is stale). **`unlink()` the file and proceed to `bind()`.**
4. If `connect()` **fails with `ENOENT`**: The file doesn't exist. **Proceed to `bind()`.**

### The Lockfile Supervisor Pattern
To avoid race conditions between two starting daemons both trying to `unlink` the same stale socket:
1. Open a separate `.lock` file.
2. Use `flock()` or `fcntl()` to acquire an **exclusive advisory lock**.
3. Only the process that holds the lock is allowed to `unlink()` the socket file and call `bind()`.
4. The kernel automatically releases the advisory lock on process death, even after `SIGKILL`.

## 4. How Other Daemons Handle This

- **ssh-agent / gpg-agent:** Use the "Connect-then-Unlink" pattern. They often create a dedicated subdirectory in `/tmp` or `$TMPDIR` with restricted permissions (`0700`) to mitigate squatting risks.
- **Docker:** Uses a combination of a persistent socket path and systemd socket activation (on Linux) or a hypervisor-managed socket (on macOS).
- **systemd:** Uses "Socket Activation" where the init system opens the socket and passes the file descriptor to the daemon. This ensures the socket is always "alive" even if the daemon restarts.

## 5. Recommendations for rustdesk-cli

1. **Update `cleanup_stale_daemon_artifacts`:** It must explicitly handle the case where `SOCKET_PATH` exists but `is_socket_alive()` is false, **regardless** of whether the lock file exists.
2. **Standardize on `$TMPDIR`:** On macOS, prefer using a path relative to `$TMPDIR` if it fits within the 104-character limit. If not, use `~/.local/share/rustdesk-cli/` or similar.
3. **Consistent Probing:** The `is_daemon_running()` check should rely primarily on `connect()` success rather than just the existence of a file.
