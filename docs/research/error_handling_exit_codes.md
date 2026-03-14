# Research: Error Handling and Exit Code Propagation

This document investigates how `rustdesk-cli` manages errors and maps them to process exit codes, compares this with the official client, and proposes a robust standard for the CLI.

## 1. Current Exit Code Mapping in `rustdesk-cli`

The current codebase (`src/main.rs`) uses a set of constants to categorize failures:

| Constant | Value | Categorized Errors |
| :--- | :--- | :--- |
| `EXIT_SUCCESS` | 0 | Command completed successfully. |
| `EXIT_CONNECTION` | 1 | hbbs/hbbr connectivity issues, timeouts, or socket failures. |
| `EXIT_SESSION` | 2 | Protocol-level errors inside an active encrypted stream. |
| `EXIT_INPUT` | 3 | Invalid CLI arguments, missing parameters, or mutually exclusive flags. |
| `EXIT_PERMISSION` | 4 | "No permission of terminal", wrong server key, or sandbox violations. |

### Error Trace Examples:
- **Connection Fail:** If `connect` times out or fails to reach hbbs, `EXIT_CONNECTION` (1) is returned.
- **Auth Fail:** If the password is wrong, the server returns a login error which is currently treated as `EXIT_SESSION` (2) or a general error category.
- **Sandbox:** If the user tries to connect to an unauthorized ID, `EXIT_PERMISSION` (4) is triggered.

## 2. Official RustDesk Client Behavior

Research into the official `rustdesk/rustdesk` source code reveals that it does **not** maintain a centralized functional exit code table for CLI usage.

- **Usage of `process::exit`**: The official client uses `std::process::exit` primarily for internal process management (e.g., restarting the server task, handling sudo prompts on Linux, or closing the UI).
- **Exit Code 0**: Most "successful" CLI commands (like setting a config value) return 0.
- **Inconsistency**: Because the official tool is primarily a GUI application, its CLI entry points often just trigger UI actions and exit immediately with 0, or exit with -1/1 on fatal startup crashes.

## 3. Proposed Exit Code Table for `rustdesk-cli`

To ensure `rustdesk-cli` is "agent-friendly" and predictable for scripts, we propose the following expanded exit code map:

| Exit Code | Name | Description |
| :--- | :--- | :--- |
| **0** | `SUCCESS` | Everything worked perfectly. |
| **1** | `GENERAL_ERROR` | Uncategorized failure or unexpected panic. |
| **2** | `CONNECT_FAILED` | Network unreachable, hbbs/hbbr down, or DNS failure. |
| **3** | `AUTH_FAILED` | Wrong password or invalid authentication token. |
| **4** | `PERMISSION_DENIED` | Server-side permission bit missing or sandbox restriction. |
| **5** | `TIMEOUT` | Command or connection exceeded the specified deadline. |
| **6** | `TRANSFER_ERROR` | File I/O error or interrupted push/pull. |
| **128+N** | `REMOTE_EXIT` | Used for `exec` to propagate remote shell exit codes (see below). |

## 4. Remote Exit Code Propagation (`exec`)

A critical requirement for AI agents is knowing if a remote command (like `cargo build`) succeeded on the peer.

### Current Flow:
1. **Sentinel Capture**: `rustdesk-cli exec` appends `echo __sentinel__$?` to the command.
2. **Parsing**: The daemon's `parse_exec_output` finds the sentinel string and extracts the digits ($?) as an `i32`.
3. **Transport**: This value is returned to the local CLI process via the UDS `SessionResponse`.
4. **Current Gap**: The local CLI currently returns `EXIT_SUCCESS` (0) as long as the UDS communication worked, ignoring the remote value.

### Proposed Flow:
- If the remote command returned non-zero (e.g., 1), `rustdesk-cli` should exit locally with that same code, or a mapped version (e.g., `128 + remote_code`) to distinguish remote failures from local transport failures.
- **Recommendation**: Directly propagate the remote exit code if it's between 1-127. If the remote process was killed by a signal (e.g., SIGKILL), use the standard shell convention of `128 + signal_number`.

## Conclusion

The transition from a GUI-first mentality to a CLI-first tool requires rigorous exit code management. By adopting the proposed table and ensuring `exec` correctly propagates remote results, we make `rustdesk-cli` a reliable component for automated inference pipelines.
