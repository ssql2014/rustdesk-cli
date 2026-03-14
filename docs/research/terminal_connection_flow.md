# Terminal Connection Flow Research

This document describes how the RustDesk protocol handles terminal-specific connections (`ConnType::TERMINAL`).

## 1. Connection Initiation

### PunchHoleRequest & RequestRelay
Both the initial `PunchHoleRequest` (sent to `hbbs` via UDP or TCP) and the subsequent `RequestRelay` (sent to `hbbs` via TCP) must specify the connection type.

- **Field:** `conn_type`
- **Value:** `ConnType::TERMINAL` (Enum value `5`)
- **Impact:** This tells the rendezvous server and the target peer that this session is intended for terminal access rather than full desktop sharing.

## 2. Login Flow for Terminal Sessions

The terminal login flow follows the standard authentication sequence but with specific modifications to the message content.

### Authentication (NaCl Handshake)
The handshake and key exchange remain identical to other connection types.

### handle_hash Phase
In the `handle_hash` logic (found in `src/client.rs`):
- If `conn_type == ConnType::TERMINAL`, the client explicitly clears the OS-level credentials (`os_username` and `os_password`).
- These are sent as empty strings in the `LoginRequest`.

### LoginRequest Structure
The `LoginRequest` message utilizes the `terminal` union variant:
- **Field 16:** `Terminal terminal`
- **Sub-fields:**
    - `service_id`: A string ID used to reconnect to a previously established persistent session. If empty, a new session is created.
- **Options:** The `OptionMessage` (field 6) may include `terminal_persistent = BoolOption::Yes` (field 18) to request that the server keeps the shell alive after this client disconnects.

## 3. Post-Login Sequence

Once `LoginResponse(PeerInfo)` is received:

1.  **Terminal Support Check:** The client verifies `pi.features.terminal` is true.
2.  **OpenTerminal:** The client sends a `TerminalAction` message containing the `OpenTerminal` variant.
    - **Fields:** `terminal_id` (usually 0 for the first one), `rows`, and `cols`.
3.  **TerminalOpened:** The server responds with `TerminalResponse(TerminalOpened)` containing the `pid` and the `service_id` (which should be saved for future reconnections).
4.  **Streaming:** The session transitions into the data streaming phase using `TerminalAction(TerminalData)` and `TerminalResponse(TerminalData)`.

## Summary of Protocol Differences

| Phase | Desktop Connection | Terminal Connection |
| :--- | :--- | :--- |
| **PunchHole/Relay** | `conn_type: DEFAULT_CONN` | `conn_type: TERMINAL` |
| **LoginRequest Union** | (None or specific) | `terminal: Terminal { ... }` |
| **OS Credentials** | Sent if available | **Always cleared** (empty strings) |
| **Initial Action** | Server starts `video_service` | Client sends `OpenTerminal` |
| **Data Messages** | `VideoFrame`, `MouseEvent` | `TerminalAction`, `TerminalResponse` |
