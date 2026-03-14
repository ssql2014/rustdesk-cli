# Terminal Peer Requirements Research

This document outlines the configuration and runtime requirements on the RustDesk peer (the controlled side) to accept terminal connections.

## 1. Peer Configuration Settings

The terminal feature is governed by a specific configuration key on the peer side.

- **Key:** `enable-terminal` (defined as `keys::OPTION_ENABLE_TERMINAL`).
- **Default Value:** **Enabled**. In RustDesk, `enable-` prefixed options are considered `true` unless explicitly set to `"N"`.
- **UI Location:** Settings -> Security -> Permissions -> "Enable terminal".

## 2. Access Control and Permissions

Terminal access is treated as a first-class permission, similar to keyboard/mouse or file transfer.

- **Permission Bit:** `Permission::terminal`.
- **Validation:** During the `LoginRequest` handling, the server checks if the terminal permission is granted:
  ```rust
  if !Self::permission(keys::OPTION_ENABLE_TERMINAL, &self.control_permissions) {
      self.send_login_error("No permission of terminal").await;
      return false;
  }
  ```
- **Control Permissions:** If the client was invited or has restricted permissions (e.g., via a customized client or server-side policy), the `control_permissions` bitmask must have the `terminal` bit (index 6) set.

## 3. Connection Approval Flow

How a terminal connection is accepted depends on the client's provided credentials and the peer's `approve-mode`.

### Scenario A: Valid Password Provided
If the client provides a password hash that matches the peer's **Permanent Password** or **Temporary Password**:
- The connection is **authorized immediately**.
- No user intervention is required on the peer side.
- This is the ideal flow for automated CLI access.

### Scenario B: No Password (Empty)
If the client sends an empty password:
- The peer triggers the **"Incoming Connection"** dialog.
- A user on the peer side must manually click **"Accept"**.
- The connection stays in a pending state until accepted or timed out.

### Scenario C: OS Credentials
For terminal sessions, the official client clears OS credentials (`os_username`, `os_password`). 
- If the peer is running as a service (installed), the terminal typically runs as the **Current Logon User**.
- If no user is logged on, the connection may fail with: *"No active console user logged on, please connect and logon first."*

## 4. Requirements Summary Table

| Requirement | Value / Condition | Note |
| :--- | :--- | :--- |
| **enable-terminal** | `!= "N"` | Must be enabled in peer settings. |
| **Verification Method** | `use-both-passwords` | Default. Allows both perm and temp passwords. |
| **Approve Mode** | `Both` or `Password` | `Click` mode would require manual acceptance even with password. |
| **Service Status** | Installed/Running | Required for "SelfUser" or "CurrentLogonUser" shell spawning. |

## Conclusion for rustdesk-cli

To achieve non-interactive terminal access, `rustdesk-cli` must:
1.  Ensure the peer has `Enable terminal` checked.
2.  Provide the correct **Permanent** or **Temporary** password in the `LoginRequest`.
3.  Handle the case where the peer is at the login screen (Windows), which may require full desktop login first before a terminal can be spawned.
