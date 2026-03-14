# Research: File Transfer Session Initialization

This document details how the RustDesk protocol initializes dedicated file transfer sessions and the corrected message flow for pushing (uploading) files.

## 1. Connection Type (ConnType)

File transfer is a unique session type in the RustDesk protocol.

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
1. **Connect:** Establish TCP to peer/relay with `ConnType::FILE_TRANSFER (1)`.
2. **Handshake:** Perform NaCl KeyExchange.
3. **Login:** Send `LoginRequest` with the `file_transfer` union variant populated.
4. **Authorization:** Wait for `LoginResponse(PeerInfo)`. 
    - Note: If no password was provided, the peer will trigger its CM UI for manual "Accept/Deny".

## 4. Corrected Initiation Flow

### Push (Upload) Flow
To "Push" a file from the CLI to the peer:
1. **Client Action:** Client sends `FileAction::Receive(FileTransferReceiveRequest)`.
    - **Logic:** The client is requesting that the *peer* "receive" a file.
    - **Metadata:** Includes `path` (remote destination), `files` (list of entries), and `id` (job ID).
2. **Peer Confirmation:** The peer responds with `FileResponse::Digest` to indicate its current state of that file.
3. **Data:** Client sends `FileResponse::Block` messages.

### Pull (Download) Flow
To "Pull" a file from the peer to the local CLI:
1. **Client Action:** Client sends `FileAction::Send(FileTransferSendRequest)`.
    - **Logic:** The client is asking the *peer* to "send" a file.
2. **Peer Response:** Peer responds with `FileResponse::Digest` containing file metadata.
3. **Client Confirmation:** Client sends `FileAction::SendConfirm` with an `OffsetBlk`.
4. **Data:** Peer starts sending `FileResponse::Block` messages.

## 5. Summary Table

| Feature | Desktop Session | Terminal Session | File Transfer Session |
| :--- | :--- | :--- | :--- |
| **ConnType** | `0` (DEFAULT) | `5` (TERMINAL) | `1` (FILE_TRANSFER) |
| **Login Variant** | (None) | `terminal` (16) | `file_transfer` (17) |
| **Multiplexing** | Shared | Usually Shared | **Always Separate** |
| **CM Approval** | Required | Required | Required |

## Implementation Notes for Leo

To fix the `push` command:
1.  Initiate a completely new connection with `ConnType::FILE_TRANSFER`.
2.  Populate field 17 in the `LoginRequest`.
3.  Ensure the initial action is `FileAction::Receive` for uploads.
