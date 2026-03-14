# RustDesk File Transfer Protocol Research

This document outlines the implementation details of the RustDesk file transfer protocol, specifically for "pushing" files from a client to a peer.

## 1. Protobuf Message Structure

The file transfer system is built on two primary `RendezvousMessage` union variants (indices 17 and 18).

### FileAction (Client to Server)
- **`ReadDir`**: List files in a directory.
- **`FileTransferSendRequest`**: Initiate a push transfer.
- **`FileTransferReceiveRequest`**: Initiate a pull transfer.
- **`FileTransferSendConfirmRequest`**: Confirm transfer after digest check (supports `Skip` or `OffsetBlk`).
- **`FileTransferCancel`**: Abort an active job.

### FileResponse (Server to Client)
- **`FileDirectory`**: Result of a `ReadDir`.
- **`FileTransferBlock`**: A chunk of file data.
- **`FileTransferDigest`**: Metadata for resume/overwrite checks (`file_size`, `last_modified`, `transferred_size`).
- **`FileTransferDone`**: Signals completion of a file.
- **`FileTransferError`**: Reports I/O or protocol errors.

## 2. Chunked Transfer Protocol

### Block Management
- **Block Size**: 128 KB (`131,072` bytes) per chunk.
- **Sequencing**: Each block is wrapped in `FileTransferBlock`. 
    - `id`: The Job ID.
    - `file_num`: Index of the file within the job.
    - `data`: The (possibly compressed) chunk bytes.
    - `blk_id`: Currently unused (defaults to 0) in the official implementation.

### Flow Control
The protocol does **not** use per-block acknowledgments. Instead:
1. The sender iterates through the file in 128KB increments.
2. It sends one block per "tick" of its internal I/O loop.
3. It relies on the underlying TCP window and socket backpressure to manage throughput.

## 3. Initiation and Negotiation Flow

To push a file (e.g., model weights):
1. **Request**: Client sends `FileAction::Send(FileTransferSendRequest)` with the path and file count.
2. **Digest**: Peer responds with `FileResponse::Digest`. This contains the peer's existing file info (if any).
3. **Confirmation**: Client compares the digest.
    - If resuming: Sends `FileAction::SendConfirm` with `OffsetBlk`.
    - If overwriting: Sends `FileAction::SendConfirm` with `OffsetBlk(0)`.
    - If skipping: Sends `FileAction::SendConfirm` with `Skip(true)`.
4. **Data Transmission**: Only after confirmation does the sender start firing `FileTransferBlock` messages.

## 4. Encryption and Compression

- **Encryption**: File data is part of the `Message` union and is thus automatically encrypted by the session's **XSalsa20-Poly1305** layer.
- **Compression**: 
    - Algorithm: **Zstd**.
    - Logic: The sender attempts to compress each 128KB chunk. If the compressed size is smaller than the original, it sends the compressed bytes and sets the `compressed` flag to `true`.
    - Skip: Certain file extensions (e.g., .zip, .png, .jpg) are typically skipped for compression.

## 5. Progress Tracking

Progress is tracked using three metrics in the `TransferJob` struct:
- **`total_size`**: Total bytes of all files in the job.
- **`finished_size`**: Total **uncompressed** bytes successfully read from source or written to disk.
- **`transferred`**: Total **compressed** bytes sent over the network.

## Implementation Notes for rustdesk-cli

For deploying `.safetensors` or `.gguf` files:
1. Use a single-file `TransferJob`.
2. Implement the `Digest` response handler to support resuming large transfers if the connection drops.
3. Ensure the `FileTransferBlock` data is compressed with Zstd before sending to minimize bandwidth usage for these typically large files.
