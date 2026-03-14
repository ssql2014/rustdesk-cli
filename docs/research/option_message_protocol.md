# OptionMessage Protocol Research

This document details the structure and usage of `OptionMessage` within the RustDesk protocol, based on analysis of the official client (`rustdesk/rustdesk`).

## 1. Role and Lifecycle

`OptionMessage` is used to communicate client-side preferences and capabilities to the peer.

- **Encrypted Only:** It is never sent via UDP/Rendezvous. It is always part of the encrypted payload after Step 4 of the handshake.
- **Initial Negotiation:** Sent within the `LoginRequest` (Field 6).
- **Dynamic Updates:** Can be sent during an active session via `Message.Misc.option` to reflect UI changes (e.g., user toggles "View Only").

## 2. Structure for Terminal Sessions

Analysis of `src/client.rs:get_option_message` reveals that the official client treats `ConnType::TERMINAL` as a special case with a highly reduced option set.

### Terminal-Only Fields:
- **`terminal_persistent`** (Field 18):
    - `BoolOption::Yes`: Request the server to keep the shell alive after this connection closes.
    - `BoolOption::NotSet`: Default behavior.

### Omitted for Terminal:
For terminal sessions, the following fields are **not set** (omitted/default):
- `image_quality` / `custom_image_quality`
- `supported_decoding` (No video codecs needed)
- `disable_audio` / `disable_camera`
- `show_remote_cursor` / `follow_remote_cursor`
- `enable_file_transfer`

## 3. Structure for Desktop Sessions

For `ConnType::DEFAULT_CONN`, the message is comprehensive:

| Field | Purpose | Typical Value |
| :--- | :--- | :--- |
| `image_quality` | Preset levels | `Best`, `Balanced`, `Low` |
| `supported_decoding` | Client hardware abilities | `ability_vp9: 1`, `prefer: Auto` |
| `disable_audio` | Client-side mute | `BoolOption::No` (default) |
| `disable_clipboard` | Prevent sync | `BoolOption::No` (default) |
| `custom_fps` | Frame rate limit | `30` or `60` |

## 4. Protobuf Definition (Reference)

```protobuf
message OptionMessage {
  enum BoolOption { NotSet = 0; No = 1; Yes = 2; }
  ImageQuality image_quality = 1;
  BoolOption lock_after_session_end = 2;
  BoolOption show_remote_cursor = 3;
  BoolOption privacy_mode = 4;
  BoolOption block_input = 5;
  int32 custom_image_quality = 6;
  BoolOption disable_audio = 7;
  BoolOption disable_clipboard = 8;
  BoolOption enable_file_transfer = 9;
  SupportedDecoding supported_decoding = 10;
  int32 custom_fps = 11;
  BoolOption disable_keyboard = 12;
  BoolOption follow_remote_cursor = 15;
  BoolOption follow_remote_window = 16;
  BoolOption disable_camera = 17;
  BoolOption terminal_persistent = 18;
  BoolOption show_my_cursor = 19;
}
```

## 5. Recommendation for rustdesk-cli

To ensure maximum compatibility and avoid the "hang after SignedId" race conditions:

1.  **Refactor `OptionMessage` Construction:** Create two distinct builders: `build_desktop_options()` and `build_terminal_options()`.
2.  **Terminal Mode:** Use `build_terminal_options()` which only sets `terminal_persistent`. This avoids sending heavy/unused codec capability fields that might trigger unwanted state transitions on the peer.
3.  **Daemon Flow:** The daemon currently uses a "priming" desktop session. This is valid, but the subsequent terminal session should use the minimal terminal-specific options.
