//! Transport abstraction for RustDesk message exchange.
//!
//! Uses RustDesk's BytesCodec framing: variable-length header (1-4 bytes)
//! where the low 2 bits of the first byte encode header size.

use anyhow::{Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// Async transport interface for sending and receiving protocol payloads.
#[allow(async_fn_in_trait)]
pub trait Transport: Sized {
    async fn connect(addr: &str) -> Result<Self>;
    async fn send(&mut self, msg: &[u8]) -> Result<()>;
    async fn recv(&mut self) -> Result<Vec<u8>>;
    async fn close(&mut self) -> Result<()>;
}

/// Raw TCP transport used to connect to a RustDesk peer.
pub struct TcpTransport {
    inner: FramedTransport<TcpStream>,
}

impl TcpTransport {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            inner: FramedTransport::new(stream),
        }
    }
}

impl Transport for TcpTransport {
    async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self::new(stream))
    }

    async fn send(&mut self, msg: &[u8]) -> Result<()> {
        self.inner.send(msg).await
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        self.inner.recv().await
    }

    async fn close(&mut self) -> Result<()> {
        self.inner.close().await
    }
}

/// RustDesk BytesCodec framing on top of an async stream.
///
/// Wire format: variable-length header (1-4 bytes LE) where the low 2 bits
/// encode `header_size - 1`:
///   0b00 → 1 byte,  max payload 63
///   0b01 → 2 bytes, max payload 16383
///   0b10 → 3 bytes, max payload 4194303
///   0b11 → 4 bytes, max payload 1073741823
/// Payload length = header_value >> 2.
pub struct FramedTransport<S> {
    stream: S,
}

impl<S> FramedTransport<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> FramedTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn send(&mut self, msg: &[u8]) -> Result<()> {
        let len = msg.len();
        if len <= 0x3F {
            // 1-byte header
            let b = ((len as u8) << 2) | 0b00;
            self.stream.write_all(&[b]).await?;
        } else if len <= 0x3FFF {
            // 2-byte header LE
            let val = ((len as u16) << 2) | 0b01;
            self.stream.write_all(&val.to_le_bytes()).await?;
        } else if len <= 0x3F_FFFF {
            // 3-byte header LE (write as u32 LE, take first 3 bytes)
            let val = ((len as u32) << 2) | 0b10;
            let bytes = val.to_le_bytes();
            self.stream.write_all(&bytes[..3]).await?;
        } else if len <= 0x3FFF_FFFF {
            // 4-byte header LE
            let val = ((len as u32) << 2) | 0b11;
            self.stream.write_all(&val.to_le_bytes()).await?;
        } else {
            bail!("payload too large for BytesCodec: {len} bytes");
        }
        self.stream.write_all(msg).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        // Read first byte to determine header size.
        let mut first = [0_u8; 1];
        self.stream.read_exact(&mut first).await?;
        let tag = first[0] & 0x03;
        let header_len = (tag + 1) as usize;

        // Read remaining header bytes (if any).
        let mut raw = [0_u8; 4];
        raw[0] = first[0];
        if header_len > 1 {
            self.stream
                .read_exact(&mut raw[1..header_len])
                .await?;
        }

        // Assemble LE integer and extract payload length.
        let combined = u32::from_le_bytes(raw);
        let payload_len = (combined >> 2) as usize;

        if payload_len > 64 * 1024 * 1024 {
            bail!("payload length {payload_len} exceeds 64 MiB safety limit");
        }

        let mut payload = vec![0_u8; payload_len];
        self.stream.read_exact(&mut payload).await?;
        Ok(payload)
    }

    pub async fn close(&mut self) -> Result<()> {
        self.stream.shutdown().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn framed_transport_roundtrip_over_duplex() -> Result<()> {
        let (client_stream, server_stream) = duplex(1024);
        let mut client = FramedTransport::new(client_stream);
        let mut server = FramedTransport::new(server_stream);

        let client_task = tokio::spawn(async move {
            client.send(b"hello").await?;
            let reply = client.recv().await?;
            client.close().await?;
            Result::<Vec<u8>>::Ok(reply)
        });

        let server_task = tokio::spawn(async move {
            let request = server.recv().await?;
            server.send(b"world").await?;
            server.close().await?;
            Result::<Vec<u8>>::Ok(request)
        });

        let request = server_task.await.expect("server task should join")?;
        let reply = client_task.await.expect("client task should join")?;

        assert_eq!(request, b"hello");
        assert_eq!(reply, b"world");
        Ok(())
    }

    #[tokio::test]
    async fn bytescodec_1byte_header() -> Result<()> {
        // Payload <= 63 bytes → 1-byte header
        let (client_stream, server_stream) = duplex(1024);
        let mut client = FramedTransport::new(client_stream);
        let mut server = FramedTransport::new(server_stream);

        let msg = b"short";
        let client_task = tokio::spawn(async move {
            client.send(msg).await?;
            Result::<()>::Ok(())
        });
        let server_task = tokio::spawn(async move {
            let data = server.recv().await?;
            Result::<Vec<u8>>::Ok(data)
        });

        client_task.await??;
        let received = server_task.await??;
        assert_eq!(received, msg);
        Ok(())
    }

    #[tokio::test]
    async fn bytescodec_2byte_header() -> Result<()> {
        // Payload 64-16383 bytes → 2-byte header
        let (client_stream, server_stream) = duplex(32768);
        let mut client = FramedTransport::new(client_stream);
        let mut server = FramedTransport::new(server_stream);

        let msg = vec![0xAB_u8; 200];
        let msg_clone = msg.clone();
        let client_task = tokio::spawn(async move {
            client.send(&msg_clone).await?;
            Result::<()>::Ok(())
        });
        let server_task = tokio::spawn(async move {
            let data = server.recv().await?;
            Result::<Vec<u8>>::Ok(data)
        });

        client_task.await??;
        let received = server_task.await??;
        assert_eq!(received, msg);
        Ok(())
    }

    #[tokio::test]
    async fn bytescodec_large_payload() -> Result<()> {
        // Payload > 16383 bytes → 3-byte header
        let (client_stream, server_stream) = duplex(128 * 1024);
        let mut client = FramedTransport::new(client_stream);
        let mut server = FramedTransport::new(server_stream);

        let msg = vec![0xCD_u8; 20000];
        let msg_clone = msg.clone();
        let client_task = tokio::spawn(async move {
            client.send(&msg_clone).await?;
            Result::<()>::Ok(())
        });
        let server_task = tokio::spawn(async move {
            let data = server.recv().await?;
            Result::<Vec<u8>>::Ok(data)
        });

        client_task.await??;
        let received = server_task.await??;
        assert_eq!(received, msg);
        Ok(())
    }
}
