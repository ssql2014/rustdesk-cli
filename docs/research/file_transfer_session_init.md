# Research: File Transfer Session Initialization

This document details how the RustDesk protocol initializes dedicated file transfer sessions, distinct from desktop or terminal sessions.

## 1. Connection Type (ConnType)

File transfer is a first-class session type in the RustDesk protocol.

- **Enum Name:** `FILE_TRANSFER`
- **Integer Value:** `1`
- **Usage:** This value must be used in the `PunchHoleRequest` (Phase 2) and `RequestRelay` (Phase 3) messages to notify the rendezvous server and the peer of the session's purpose.

## 2. Dedicated Transport

The official RustDesk client **always opens a separate TCP connection** for file transfer. 
- It does not share the same encrypted stream as an active desktop or terminal session.
- This allows file transfers to have their own flow control and not block UI/terminal input.

## 3. Login Sequence

The handshake (Phase 4) is identical to other sessions, but the `LoginRequest` (Step 7) is customized.

### LoginRequest Structure for File Transfer:
```protobuf
message LoginRequest {
  // ... common fields (username, my_id, etc.) ...
  
  // Field 17: Dedicated FileTransfer variant
  FileTransfer file_transfer = 17; 
}

message FileTransfer {
  string dir = 1;        // Initial remote directory to browse
  bool show_hidden = 2;  // Whether to show hidden files in the UI
}
```

### Protocol Sequence:
1. **Connect:** Establish TCP to peer/relay with `ConnType::FILE_TRANSFER`.
2. **Handshake:** Perform NaCl KeyExchange.
3. **Login:** Send `LoginRequest` with the `file_transfer` union variant populated.
4. **Authorization:** Wait for `LoginResponse(PeerInfo)`. 
    - Note: If no password was provided, the peer will trigger its CM UI for manual "Accept/Deny".

## 4. Push (Upload) Initiation Flow

Leo's discovery regarding `ReceiveRequest` is confirmed by the official implementation. To "Push" a file from the CLI to the peer:

1. **Client Action:** Client sends `FileAction::Receive(FileTransferReceiveRequest)`.
    - Note the terminology: The client is requesting that the *peer* "receive" a file.
2. **Digest:** The client must provide a `FileEntry` list and a `FileTransferDigest` for the file(s) being pushed.
3. **Peer Confirmation:** The peer responds with a `FileResponse::Digest` indicating how much of the file it already has (for resuming).
4. **Data:** Client sends `FileResponse::Block` messages.

## 5. Summary Table

| Feature | Desktop Session | Terminal Session | File Transfer Session |
| :--- | :--- | :--- | :--- |
| **ConnType** | `0` (DEFAULT) | `5` (TERMINAL) | `1` (FILE_TRANSFER) |
| **Login Variant** | (None) | `terminal` (16) | `file_transfer` (17) |
| **Multiplexing** | Shared | Usually Shared | **Always Separate** |
| **CM Approval** | Required | Required | Required |

## Implementation Notes for rustdesk-cli

To implement `rustdesk-cli push` robustly:
1.  Initiate a completely new connection with `ConnType::FILE_TRANSFER`.
2.  Populate field 17 in the `LoginRequest`.
3.  Follow the "Peer-Receives" flow: Send `FileTransferReceiveRequest` with a pre-calculated digest of the local file.
