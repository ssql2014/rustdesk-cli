//! Transport abstraction for RustDesk message exchange.

use anyhow::Result;
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

/// Adds RustDesk-style length-prefix framing on top of an async stream.
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
        let len = u32::try_from(msg.len())?;
        self.stream.write_all(&len.to_le_bytes()).await?;
        self.stream.write_all(msg).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut header = [0_u8; 4];
        self.stream.read_exact(&mut header).await?;
        let len = u32::from_le_bytes(header) as usize;

        let mut payload = vec![0_u8; len];
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
}
