# Research: Pull File Transfer Protocol

This document details the protocol flow for downloading files from a remote peer (pull) using the RustDesk protocol.

## 1. Initiation and Direction

In the RustDesk file transfer protocol, the direction is defined relative to the server (peer).

- **"Send" (Server $\to$ Client):** Used for **Pulling** (Downloading).
- **"Receive" (Client $\to$ Server):** Used for **Pushing** (Uploading).

To initiate a download, the client must send a `FileAction::Send(FileTransferSendRequest)`.

## 2. Protocol Sequence (Pull)

The download flow follows a 5-step negotiation:

1. **Request (Client $\to$ Server):**
   - Message: `FileAction::Send(FileTransferSendRequest)`
   - Fields: `path` (remote file/dir), `id` (new job ID), `file_num` (0 for single file).
2. **Digest (Server $\to$ Client):**
   - Message: `FileResponse::Digest`
   - Fields: `files` (list of `FileEntry` with size/time), `id` (job ID).
   - Purpose: Allows the client to see what the server has before committing to the download.
3. **Confirmation (Client $\to$ Server):**
   - Message: `FileAction::SendConfirm(FileTransferSendConfirmRequest)`
   - Union: `OffsetBlk` (the byte offset to start from) or `Skip`.
   - Purpose: Trigger the actual data flow. For a new download, use `OffsetBlk(0)`.
4. **Data Transmission (Server $\to$ Client):**
   - Message: `FileResponse::Block(FileTransferBlock)`
   - Sequence: Server sends multiple 128KB chunks.
5. **Completion (Server $\to$ Client):**
   - Message: `FileResponse::Done(FileTransferDone)`
   - Fields: `file_num`.

## 3. Resume Support

Interrupted downloads can be resumed using the same mechanism as uploads:
1. The client checks the size of its local partial file.
2. After receiving the `FileResponse::Digest` from the server, the client verifies the remote file hasn't changed (size/mtime).
3. The client sends `SendConfirm` with an `OffsetBlk` equal to the local file size.
4. The server seeks to that position and resumes sending blocks.

## 4. Implementation for rustdesk-cli

### Proposed CLI Command
```bash
rustdesk-cli pull --peer 308235080 /home/evas/output.json ./local_results/
```

### Logical Flow
1. **Connect:** Establish a dedicated TCP connection with `ConnType::FILE_TRANSFER (1)`.
2. **Phase 1 (Meta):** Send `FileAction::Send`.
3. **Phase 2 (Check):** Wait for `FileResponse::Digest`. Check local disk for existing partial file.
4. **Phase 3 (Confirm):** Send `FileAction::SendConfirm` with appropriate offset.
5. **Phase 4 (Recv):** Loop on `encrypted.recv()`.
   - If `FileResponse::Block`, write to local file and update progress.
   - If `FileResponse::Done`, close file and exit.
   - If `FileResponse::Error`, log and exit with `EXIT_TRANSFER (6)`.

## 5. Use Case: AI Inference Results

Pulling is critical for retrieving large-scale results from remote inference runs:
- **Tensors:** Downloading `.npy` or `.bin` output files.
- **Logs:** Retrieving full execution traces that exceed the 4KB PTY limit.
- **Artifacts:** Grabbing generated images or model checkpoints.
