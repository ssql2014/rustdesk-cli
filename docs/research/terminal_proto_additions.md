# Protobuf Messages for Terminal Session Support

This document identifies the Protobuf messages required for terminal session support in `rustdesk-cli`.

## 1. Required Message Types

The following message types from the official RustDesk protocol are essential for the terminal session flow:

### Terminal-Specific Messages (in `message.proto`)
- `OpenTerminal`: Initiates a new or reconnects to an existing terminal session.
- `ResizeTerminal`: Updates the rows and columns of the remote PTY.
- `TerminalData`: Streams raw bytes (input from client, output from server).
- `CloseTerminal`: Closes the terminal session.
- `TerminalAction`: A `oneof` wrapper for the above client-to-server actions.
- `TerminalOpened`: Server's response to an `OpenTerminal` request.
- `TerminalClosed`: Notification that a terminal has exited.
- `TerminalError`: Notification of a terminal-related failure.
- `TerminalResponse`: A `oneof` wrapper for the above server-to-client responses.

### Enums and Variants
- `ConnType::TERMINAL` (Value: `5`): Used in `LoginRequest` and `PunchHoleRequest` to specify the session type.
- `LoginRequest` union variant: `Terminal terminal = 16;`.
- `Message` union variants:
    - `TerminalAction terminal_action = 31;`
    - `TerminalResponse terminal_response = 32;`
- `OptionMessage`: `BoolOption terminal_persistent = 18;`

## 2. Comparison with Current Project

An audit of the current `proto/message.proto` and `proto/rendezvous.proto` reveals that **all required terminal session messages are already present**.

| Message/Enum | Status in `rustdesk-cli` |
| :--- | :--- |
| `OpenTerminal`, `ResizeTerminal`, `TerminalData`, `CloseTerminal` | Present |
| `TerminalAction`, `TerminalResponse` | Present |
| `TerminalOpened`, `TerminalClosed`, `TerminalError` | Present |
| `ConnType::TERMINAL` | Present (Value 5) |
| `LoginRequest` Terminal variant | Present (Field 16) |
| `Message` Terminal variants | Present (Fields 31, 32) |

## 3. Implementation Note

Since the Protobuf definitions are already complete and synchronized with the official RustDesk 1.4+ protocol, the next step is implementation in `src/session.rs` and `src/text_session.rs`. 

The client should:
1. Set `ConnType::TERMINAL` in `PunchHoleRequest`.
2. Set the `terminal` variant in `LoginRequest`.
3. Handle `TerminalResponse` messages in the message loop.
4. Send `TerminalAction` messages for input and window resizing.
