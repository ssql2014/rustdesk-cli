# Permission and Access Control Research

This document outlines how the RustDesk protocol and official client manage permissions, connection approval, and access control settings.

## 1. Connection Approval Flow

RustDesk distinguishes between "Authentication" (proving who you are via password) and "Authorization" (being granted permission to connect).

### Scenarios:
- **Valid Password:** If the client provides a correct permanent or temporary password, the server typically authorizes the connection immediately without a prompt (depending on `approve-mode`).
- **No Password / Click Mode:** The server triggers an **Incoming Connection** dialog on the controlled side.
    - The server sends an `ipc::Data::Login` message to the **Connection Manager (CM)** UI.
    - The CM shows "Accept" and "Dismiss" buttons.
    - "Accept" sends `ipc::Data::Authorize` back to the server, which then sends a successful `LoginResponse` to the client.
    - "Dismiss" sends `ipc::Data::Close`, terminating the connection attempt.

## 2. Access Control Settings

Configuration keys in `libs/hbb_common/src/config.rs` define the peer's security posture:

| Key | Values | Description |
| :--- | :--- | :--- |
| `approve-mode` | `Both`, `Password`, `Click` | Determines if manual approval is required. |
| `whitelist` | IP strings | Only allows connections from these source IPs. |
| `access-mode` | `full`, `view` | Global permission presets. |
| `enable-terminal` | `Y` / `N` | Specifically enables/disables the terminal feature. |
| `verification-method` | `use-both-passwords`, etc. | Controls which password types are accepted. |

## 3. Protocol Signaling

### Permission Negotiation
Permissions are not just checked at login; they can be negotiated and changed dynamically.

- **`ControlPermissions` (Initial):** A bitmask sent by `hbbs` to the peer in the `PunchHole` or `RequestRelay` messages. This allows a central server to restrict what a client can do before the session even starts.
- **`PermissionInfo` (Dynamic):** Sent within a `Misc` message during an active session.
    ```protobuf
    message PermissionInfo {
      enum Permission {
        Keyboard = 0;
        Clipboard = 2;
        Audio = 3;
        File = 4;
        Restart = 5;
        Recording = 6;
        BlockInput = 7;
      }
      Permission permission = 1;
      bool enabled = 2;
    }
    ```
    When a user toggles a permission in the RustDesk UI, the controlled side sends this message to the controlling side to enable/disable UI elements.

## 4. Summary for rustdesk-cli

For the implementation of `--dangerously-skip-permissions`:
1. **Client Side:** The CLI must still send a `LoginRequest`. To skip prompts, it should ideally provide a valid password.
2. **Server Side (Controlled):** If `rustdesk-cli` is acting as the controlled side, skipping permissions means auto-responding `Authorize` to any `ipc::Data::Login` request, effectively bypassing the CM UI.
3. **Bitmask Handling:** Ensure the `ControlPermissions` bitmask is handled correctly during the handshake to avoid being restricted by the rendezvous server.
