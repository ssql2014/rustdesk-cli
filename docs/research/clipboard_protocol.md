# Research: RustDesk Clipboard Protocol

This document details the clipboard synchronization mechanism used in the RustDesk protocol and evaluates its suitability as a lightweight data pipe.

## 1. Protobuf Message Structure

Clipboard data is exchanged using two primary message types in the `Message` union:

### `Clipboard`
- **`format`**: Enum (Text=0, Rtf=1, Html=2, ImageRgba=21, ImagePng=22, Special=31).
- **`content`**: Raw bytes of the clipboard data.
- **`compress`**: Boolean flag. If true, `content` is Zstd-compressed.
- **`special_name`**: Used for custom clipboard formats when `format` is `Special`.

### `MultiClipboards` (Variant index 28)
- A wrapper containing a `repeated Clipboard clipboards` field. Modern RustDesk clients prefer this to send multiple formats (e.g., plain text + HTML) in a single transaction.

## 2. Size Limitations

- **Protocol Limit**: The `BytesCodec` framing supports up to **1GB**.
- **Practical Limit**: The default implementation in `hbb_common` (and `rustdesk-cli`) imposes a **64MB** safety limit on incoming packets.
- **Comparison**: This is significantly higher than the **4KB** PTY `MAX_INPUT` limit, making it ideal for payloads between 4KB and 1MB.

## 3. Communication Flow

The clipboard protocol is **Push-based**:
1. **Monitoring**: Both client and peer run a background task that polls the local system clipboard (typically every 500ms).
2. **Detection**: If the clipboard content changes, the side constructs a `MultiClipboards` message.
3. **Transmission**: The message is sent through the encrypted session stream.
4. **Application**: The receiver extracts the content, decompresses it if necessary, and updates its own system clipboard.

*Note: There is no standard "RequestClipboard" or "Pull" message in the protocol. Synchronization is entirely event-driven by local changes.*

## 4. Programmatic Data Pipe Potential

The clipboard channel can be used as a "side-channel" for data transfer:
- **Pros**: Bypasses PTY canonical mode limits; handles binary data via the `bytes` field; built-in compression.
- **Cons**: Overwrites the user's actual clipboard on the remote machine; requires the remote side to have a listener (which the official RustDesk server always does).

## 5. Recommendation for rustdesk-cli

### `clipboard set`
We can implement this by sending a `MultiClipboards` message containing a single `Text` or `Special` format entry. 
- **Command**: `rustdesk-cli clipboard set --text "data"` or `rustdesk-cli exec --file path.txt --to-clipboard`.
- **Logic**: Encode payload -> Compress (Zstd) -> Wrap in `MultiClipboards` -> Send via `EncryptedStream`.

### `clipboard get`
Since the protocol is push-only, "getting" the clipboard requires the remote side to perform a copy.
- **Workaround**: We can run a remote command via `exec` that puts data into the clipboard (e.g., `echo "secret" | xclip -selection clipboard`), which will then trigger the remote RustDesk server to "push" the new clipboard content back to us.

## Conclusion

The clipboard protocol is a robust fallback for data payloads < 64MB. It provides an encrypted, compressed, and framed channel that is not subject to terminal input buffer limitations.
