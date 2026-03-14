use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use prost::Message as ProstMessage;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::crypto::EncryptedStream;
use crate::proto::hbb::{
    FileAction, FileResponse, FileTransferBlock, FileTransferDigest, FileTransferDone,
    FileTransferError, FileTransferSendConfirmRequest, FileTransferSendRequest, Message,
    TestDelay, file_action, file_response, file_transfer_send_confirm_request,
    file_transfer_send_request, message, misc,
};
use crate::transport::Transport;

pub const FILE_TRANSFER_BLOCK_SIZE: usize = 128 * 1024;

#[derive(Debug, Clone)]
pub struct PushProgress {
    pub sent_bytes: u64,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub resumed_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct PushResult {
    pub local_path: PathBuf,
    pub remote_path: String,
    pub total_bytes: u64,
    pub sent_bytes: u64,
    pub transferred_bytes: u64,
    pub resumed_bytes: u64,
    pub job_id: i32,
}

pub struct PushTransfer {
    local_path: PathBuf,
    remote_path: String,
    file: File,
    total_bytes: u64,
    sent_bytes: u64,
    transferred_bytes: u64,
    resumed_bytes: u64,
    job_id: i32,
}

enum FileTransferEvent {
    Digest(FileTransferDigest),
    Done(FileTransferDone),
    Error(FileTransferError),
}

impl PushTransfer {
    pub async fn begin<T: Transport>(
        stream: &mut EncryptedStream<T>,
        local_path: &Path,
        remote_path: &str,
    ) -> Result<Self> {
        let metadata = tokio::fs::metadata(local_path)
            .await
            .with_context(|| format!("reading local file metadata for {}", local_path.display()))?;
        if !metadata.is_file() {
            bail!("push supports single files only: {}", local_path.display());
        }

        let total_bytes = metadata.len();
        let job_id = new_job_id();
        send_message(
            stream,
            Message {
                union: Some(message::Union::FileAction(FileAction {
                    union: Some(file_action::Union::Send(FileTransferSendRequest {
                        id: job_id,
                        path: remote_path.to_string(),
                        include_hidden: false,
                        file_num: 1,
                        file_type: file_transfer_send_request::FileType::Generic as i32,
                    })),
                })),
            },
        )
        .await
        .context("sending file transfer request")?;

        let digest = loop {
            match recv_file_event(stream, job_id)
                .await
                .context("waiting for file transfer digest")?
            {
                FileTransferEvent::Digest(digest) => break digest,
                FileTransferEvent::Error(err) => bail!("file transfer error: {}", err.error),
                FileTransferEvent::Done(_) => continue,
            }
        };

        let resumed_bytes = if digest.is_identical {
            digest
                .transferred_size
                .min(total_bytes)
                .min(u32::MAX as u64)
        } else {
            0
        };
        let resume_offset = resumed_bytes as u32;
        send_message(
            stream,
            Message {
                union: Some(message::Union::FileAction(FileAction {
                    union: Some(file_action::Union::SendConfirm(
                        FileTransferSendConfirmRequest {
                            id: job_id,
                            file_num: digest.file_num,
                            union: Some(
                                file_transfer_send_confirm_request::Union::OffsetBlk(
                                    resume_offset,
                                ),
                            ),
                        },
                    )),
                })),
            },
        )
        .await
        .context("sending file transfer confirmation")?;

        let mut file = File::open(local_path)
            .await
            .with_context(|| format!("opening local file {}", local_path.display()))?;
        if resumed_bytes > 0 {
            file.seek(std::io::SeekFrom::Start(resumed_bytes))
                .await
                .with_context(|| {
                    format!(
                        "seeking local file {} to resume offset {resumed_bytes}",
                        local_path.display()
                    )
                })?;
        }

        Ok(Self {
            local_path: local_path.to_path_buf(),
            remote_path: remote_path.to_string(),
            file,
            total_bytes,
            sent_bytes: resumed_bytes,
            transferred_bytes: 0,
            resumed_bytes,
            job_id,
        })
    }

    pub fn progress(&self) -> PushProgress {
        PushProgress {
            sent_bytes: self.sent_bytes,
            total_bytes: self.total_bytes,
            transferred_bytes: self.transferred_bytes,
            resumed_bytes: self.resumed_bytes,
        }
    }

    pub fn result(&self) -> PushResult {
        PushResult {
            local_path: self.local_path.clone(),
            remote_path: self.remote_path.clone(),
            total_bytes: self.total_bytes,
            sent_bytes: self.sent_bytes,
            transferred_bytes: self.transferred_bytes,
            resumed_bytes: self.resumed_bytes,
            job_id: self.job_id,
        }
    }

    pub async fn send_next_block<T: Transport>(
        &mut self,
        stream: &mut EncryptedStream<T>,
    ) -> Result<bool> {
        let mut chunk = vec![0u8; FILE_TRANSFER_BLOCK_SIZE];
        let read = self
            .file
            .read(&mut chunk)
            .await
            .with_context(|| format!("reading local file {}", self.local_path.display()))?;
        if read == 0 {
            return Ok(false);
        }
        chunk.truncate(read);

        let (data, compressed) = compress_chunk(&chunk)?;
        self.sent_bytes += read as u64;
        self.transferred_bytes += data.len() as u64;

        send_message(
            stream,
            Message {
                union: Some(message::Union::FileResponse(FileResponse {
                    union: Some(file_response::Union::Block(FileTransferBlock {
                        id: self.job_id,
                        file_num: 0,
                        data,
                        compressed,
                        blk_id: 0,
                    })),
                })),
            },
        )
        .await
        .with_context(|| format!("sending file block for {}", self.local_path.display()))?;

        Ok(true)
    }

    pub async fn wait_for_done<T: Transport>(
        &self,
        stream: &mut EncryptedStream<T>,
    ) -> Result<()> {
        loop {
            match recv_file_event(stream, self.job_id)
                .await
                .context("waiting for file transfer completion")?
            {
                FileTransferEvent::Done(done) if done.file_num == 0 => return Ok(()),
                FileTransferEvent::Error(err) => bail!("file transfer error: {}", err.error),
                FileTransferEvent::Digest(_) | FileTransferEvent::Done(_) => continue,
            }
        }
    }
}

fn compress_chunk(chunk: &[u8]) -> Result<(Vec<u8>, bool)> {
    let compressed = zstd::encode_all(chunk, 3).context("zstd compression failed")?;
    if compressed.len() < chunk.len() {
        Ok((compressed, true))
    } else {
        Ok((chunk.to_vec(), false))
    }
}

async fn send_message<T: Transport>(
    stream: &mut EncryptedStream<T>,
    message: Message,
) -> Result<()> {
    let mut buf = Vec::new();
    message.encode(&mut buf)?;
    stream.send(&buf).await
}

async fn recv_file_event<T: Transport>(
    stream: &mut EncryptedStream<T>,
    job_id: i32,
) -> Result<FileTransferEvent> {
    loop {
        let raw = stream.recv().await.context("receiving file transfer message")?;
        if raw.is_empty() {
            continue;
        }
        let msg = Message::decode(raw.as_slice()).context("decoding file transfer message")?;
        match msg.union {
            Some(message::Union::FileResponse(resp)) => match resp.union {
                Some(file_response::Union::Digest(digest)) if digest.id == job_id => {
                    return Ok(FileTransferEvent::Digest(digest));
                }
                Some(file_response::Union::Done(done)) if done.id == job_id => {
                    return Ok(FileTransferEvent::Done(done));
                }
                Some(file_response::Union::Error(err)) if err.id == job_id => {
                    return Ok(FileTransferEvent::Error(err));
                }
                _ => continue,
            },
            Some(message::Union::TestDelay(td)) => {
                echo_test_delay(stream, td).await?;
            }
            Some(message::Union::Misc(misc_msg)) => {
                if let Some(misc::Union::CloseReason(reason)) = misc_msg.union {
                    bail!("peer closed file transfer session: {reason}");
                }
            }
            _ => continue,
        }
    }
}

async fn echo_test_delay<T: Transport>(
    stream: &mut EncryptedStream<T>,
    td: TestDelay,
) -> Result<()> {
    send_message(
        stream,
        Message {
            union: Some(message::Union::TestDelay(TestDelay {
                time: td.time,
                from_client: true,
                last_delay: td.last_delay,
                target_bitrate: td.target_bitrate,
            })),
        },
    )
    .await
    .context("echoing TestDelay during file transfer")
}

fn new_job_id() -> i32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_nanos();
    ((nanos % (i32::MAX as u128 - 1)) as i32) + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::EncryptedStream;
    use crate::transport::{FramedTransport, Transport};
    use std::path::PathBuf;
    use tokio::io::duplex;

    struct DuplexTransport {
        framed: FramedTransport<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn pair() -> (Self, Self) {
            let (a, b) = duplex(1024 * 1024);
            (
                Self { framed: FramedTransport::new(a) },
                Self { framed: FramedTransport::new(b) },
            )
        }
    }

    impl Transport for DuplexTransport {
        async fn connect(_addr: &str) -> Result<Self> {
            unimplemented!()
        }
        async fn send(&mut self, msg: &[u8]) -> Result<()> {
            self.framed.send(msg).await
        }
        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.framed.recv().await
        }
        async fn close(&mut self) -> Result<()> {
            self.framed.close().await
        }
    }

    fn temp_file_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("rustdesk-cli-{name}-{suffix}.bin"))
    }

    #[tokio::test]
    async fn push_transfer_resumes_and_compresses_blocks() {
        let local_path = temp_file_path("push");
        let local_bytes = vec![b'a'; 256 * 1024];
        let local_len = local_bytes.len() as u64;
        tokio::fs::write(&local_path, &local_bytes).await.unwrap();

        let (client_transport, server_transport) = DuplexTransport::pair();
        let key = [7u8; 32];
        let mut client = EncryptedStream::new(client_transport, &key);
        let mut server = EncryptedStream::new(server_transport, &key);

        let server_task = tokio::spawn(async move {
            let raw = server.recv().await.unwrap();
            let msg = Message::decode(raw.as_slice()).unwrap();
            let send_req = match msg.union.unwrap() {
                message::Union::FileAction(action) => match action.union.unwrap() {
                    file_action::Union::Send(req) => req,
                    other => panic!("unexpected send request: {other:?}"),
                },
                other => panic!("unexpected message: {other:?}"),
            };
            assert_eq!(send_req.path, "/remote/rope.bin");
            assert_eq!(send_req.file_num, 1);

            send_message(
                &mut server,
                Message {
                    union: Some(message::Union::FileResponse(FileResponse {
                        union: Some(file_response::Union::Digest(FileTransferDigest {
                            id: send_req.id,
                            file_num: 0,
                            file_size: local_len,
                            is_identical: true,
                            transferred_size: 128 * 1024,
                            ..Default::default()
                        })),
                    })),
                },
            )
            .await
            .unwrap();

            let raw = server.recv().await.unwrap();
            let msg = Message::decode(raw.as_slice()).unwrap();
            let confirm = match msg.union.unwrap() {
                message::Union::FileAction(action) => match action.union.unwrap() {
                    file_action::Union::SendConfirm(req) => req,
                    other => panic!("unexpected confirm: {other:?}"),
                },
                other => panic!("unexpected message: {other:?}"),
            };
            assert_eq!(confirm.file_num, 0);

            let raw = server.recv().await.unwrap();
            let msg = Message::decode(raw.as_slice()).unwrap();
            let block = match msg.union.unwrap() {
                message::Union::FileResponse(resp) => match resp.union.unwrap() {
                    file_response::Union::Block(block) => block,
                    other => panic!("unexpected block response: {other:?}"),
                },
                other => panic!("unexpected message: {other:?}"),
            };
            assert!(block.compressed);

            send_message(
                &mut server,
                Message {
                    union: Some(message::Union::FileResponse(FileResponse {
                        union: Some(file_response::Union::Done(FileTransferDone {
                            id: send_req.id,
                            file_num: 0,
                        })),
                    })),
                },
            )
            .await
            .unwrap();
        });

        let mut transfer = PushTransfer::begin(&mut client, &local_path, "/remote/rope.bin")
            .await
            .unwrap();
        assert_eq!(transfer.progress().resumed_bytes, 128 * 1024);
        assert!(transfer.send_next_block(&mut client).await.unwrap());
        assert!(!transfer.send_next_block(&mut client).await.unwrap());
        transfer.wait_for_done(&mut client).await.unwrap();
        let result = transfer.result();
        assert_eq!(result.sent_bytes, local_len);
        assert_eq!(result.resumed_bytes, 128 * 1024);

        server_task.await.unwrap();
        let _ = tokio::fs::remove_file(local_path).await;
    }
}
