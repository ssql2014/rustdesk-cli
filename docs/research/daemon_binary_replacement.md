# Research: Daemon Process Death on Binary Recompilation

This document investigates why the `rustdesk-cli` daemon process dies when the project is recompiled (Issue #37) and explores common patterns for handling binary replacement.

## 1. Operating System Behavior

The primary cause of process death during recompilation stems from how the kernel manages running executables, particularly on macOS.

### Unix Unlink Semantics
On most Unix-like systems (Linux, BSD), a file's name in a directory is just a link to an "inode" (the actual data on disk).
- **Unlinking:** If you `rm` or `mv` a running binary, the directory entry is removed or changed, but the inode remains alive as long as at least one process has it open. The running process continues to execute the "old" code undisturbed.
- **Overwriting:** If you overwrite the file (e.g., via `cp` or by opening it with `O_TRUNC`), you are modifying the existing inode.

### macOS Specifics (SIGBUS vs. SIGKILL)
macOS is more aggressive about protecting the integrity of running binaries:
- **SIGBUS (Bus Error):** macOS uses memory-mapped files (`mmap`) to load code pages on demand. If the binary file on disk is truncated or overwritten while the process is running, any subsequent page fault (attempting to load a new part of the code) will fail because the file contents no longer match the expected offsets. The kernel then sends `SIGBUS` to the process.
- **SIGKILL (Killed: 9):** On Apple Silicon (M1/M2/M3), macOS strictly enforces code signatures via the Apple Mobile File Integrity (AMFI) subsystem. If the binary at the current process's path is replaced with a new one (even via unlinking), the kernel may detect a signature mismatch or a "Team ID" change and immediately terminate the process with `SIGKILL`.

## 2. Binary Replacement Patterns

### The "Move Aside" Pattern
The most robust workaround for development on macOS is the "Move Aside" trick. Instead of letting the compiler/linker overwrite the binary, you rename the old one first:
```bash
mv target/debug/rustdesk-cli target/debug/rustdesk-cli.old
cargo build
```
This ensures that the running daemon continues to point to the valid `.old` inode (preserving memory-map consistency and signature validity) while `cargo` creates a brand-new file for the new version.

### The `self-replace` Crate
In the Rust ecosystem, the `self-replace` crate is commonly used to automate this. It handles the platform-specific nuances of unlinking and replacing a running executable, which is useful for "self-updating" tools.

## 3. Fork+Exec and Graceful Restarts

For long-running servers (like Nginx), "Zero-Downtime Restarts" are achieved using a `fork` + `exec` handoff:
1.  The running process (v1) receives a signal (e.g., `SIGHUP`).
2.  It calls `fork()`. The child inherits all open file descriptors, including the listening UDS/TCP socket.
3.  The child calls `execve()` on the **new** binary (v2).
4.  The new process (v2) detects it was started as an upgrade (e.g., via an environment variable) and starts accepting connections on the inherited socket.
5.  The old process (v1) gracefully shuts down.

## 4. Ecosystem Tooling

### watchexec / cargo-watch
These tools are designed for development. They monitor the filesystem and **explicitly kill** the running process before starting a new build and execution cycle. If the daemon dies during a `cargo-watch` session, it is usually because the watcher killed it to prevent the `SIGBUS` issues described above.

### systemd
On Linux, `systemd` handles binary replacement by expecting the service to be restarted (`systemctl restart`). It can use "Socket Activation" to hold the listening socket open while the binary is being swapped, ensuring no connections are lost during the transition.

## 5. Summary of Findings for rustdesk-cli

The daemon's death during recompilation is a side effect of macOS's kernel-level protections for memory-mapped executables and code signing.

- **Likely Signal:** The process is likely receiving `SIGBUS` (if the file is overwritten) or `SIGKILL: 9` (if the signature becomes invalid).
- **Blockers:** This is an OS-level behavior and cannot be "fixed" entirely within the daemon's logic if the binary file itself is being replaced at the same path.
- **Recommendation:** Document the "Move Aside" trick for developers or integrate a self-restart mechanism that can detect binary changes and perform a graceful handoff if persistence is required.
