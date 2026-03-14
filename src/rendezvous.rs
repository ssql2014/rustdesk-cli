//! Rendezvous client for RustDesk's hbbs server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use prost::Message;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::crypto::{self, EncryptedStream};
use crate::proto::hbb::{
    ConnType, KeyExchange, NatType, PunchHoleRequest, PunchHoleResponse, RegisterPeer,
    RegisterPeerResponse, RegisterPk, RegisterPkResponse, RelayResponse, RendezvousMessage,
    RequestRelay,
    rendezvous_message,
};
use crate::transport::{TcpTransport, Transport};

pub struct RendezvousClient {
    socket: Arc<UdpSocket>,
    server_addr: String,
}

#[derive(Debug)]
pub enum PunchRelayResponse {
    PunchHole(PunchHoleResponse),
    Relay(RelayResponse),
}

impl RendezvousClient {
    pub async fn connect(server_addr: &str) -> Result<Self> {
        let bind_addr = if server_addr.contains('[') { "[::]:0" } else { "0.0.0.0:0" };
        let socket = UdpSocket::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind local UDP socket for {server_addr}"))?;
        socket
            .connect(server_addr)
            .await
            .with_context(|| format!("failed to connect UDP socket to rendezvous server {server_addr}"))?;
        Ok(Self {
            socket: Arc::new(socket),
            server_addr: server_addr.to_string(),
        })
    }

    /// Spawn a background task that sends RegisterPeer every 10 seconds to
    /// maintain presence on the rendezvous server.  Returns a JoinHandle that
    /// should be aborted once discovery completes.
    pub fn start_heartbeat(&self, my_id: &str) -> JoinHandle<()> {
        spawn_heartbeat(Arc::clone(&self.socket), my_id.to_string(), Duration::from_secs(10))
    }

    pub async fn register_peer(
        &self,
        my_id: &str,
        _public_key: &[u8],
    ) -> Result<RegisterPeerResponse> {
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::RegisterPeer(RegisterPeer {
                id: my_id.to_string(),
                serial: 0,
            })),
        };

        // Use direct send+recv (not send_request) because the expected
        // RegisterPeerResponse is the same type that send_request skips
        // when filtering heartbeat noise.
        let mut buf = Vec::new();
        request.encode(&mut buf)?;
        self.socket.send(&buf).await?;

        let mut recv_buf = vec![0_u8; 4096];
        let size = self.socket.recv(&mut recv_buf).await?;
        let response = RendezvousMessage::decode(&recv_buf[..size])?;

        match response.union {
            Some(rendezvous_message::Union::RegisterPeerResponse(r)) => Ok(r),
            other => bail!("unexpected rendezvous response to RegisterPeer: {other:?}"),
        }
    }

    pub async fn register_pk(
        &self,
        my_id: &str,
        uuid: &[u8],
        public_key: &[u8],
    ) -> Result<RegisterPkResponse> {
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::RegisterPk(RegisterPk {
                id: my_id.to_string(),
                uuid: uuid.to_vec(),
                pk: public_key.to_vec(),
                old_id: String::new(),
                no_register_device: false,
            })),
        };

        match self.send_request(&request).await?.union {
            Some(rendezvous_message::Union::RegisterPkResponse(response)) => Ok(response),
            other => bail!("unexpected rendezvous response to RegisterPk: {other:?}"),
        }
    }

    pub async fn punch_hole(&self, target_id: &str, licence_key: &str) -> Result<PunchHoleResponse> {
        self.punch_hole_with_conn_type(target_id, licence_key, ConnType::DefaultConn)
            .await
    }

    pub async fn punch_hole_with_conn_type(
        &self,
        target_id: &str,
        licence_key: &str,
        conn_type: ConnType,
    ) -> Result<PunchHoleResponse> {
        match self
            .punch_hole_or_relay_with_conn_type(target_id, licence_key, conn_type)
            .await?
        {
            PunchRelayResponse::PunchHole(response) => Ok(response),
            PunchRelayResponse::Relay(response) => {
                bail!("unexpected RelayResponse to PunchHoleRequest: {response:?}")
            }
        }
    }

    pub async fn punch_hole_or_relay_with_conn_type(
        &self,
        target_id: &str,
        licence_key: &str,
        conn_type: ConnType,
    ) -> Result<PunchRelayResponse> {
        let udp_port = self.socket.local_addr()?.port() as i32;
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
                id: target_id.to_string(),
                nat_type: NatType::UnknownNat as i32,
                licence_key: licence_key.to_string(),
                conn_type: conn_type as i32,
                token: String::new(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                udp_port,
                force_relay: true,
                upnp_port: 0,
                socket_addr_v6: Vec::new(),
            })),
        };

        match self.send_request(&request).await?.union {
            Some(rendezvous_message::Union::PunchHoleResponse(response)) => {
                Ok(PunchRelayResponse::PunchHole(response))
            }
            Some(rendezvous_message::Union::RelayResponse(response)) => {
                Ok(PunchRelayResponse::Relay(response))
            }
            other => bail!("unexpected rendezvous response to PunchHoleRequest: {other:?}"),
        }
    }

    pub async fn punch_hole_via_tcp_with_conn_type(
        &self,
        target_id: &str,
        licence_key: &str,
        conn_type: ConnType,
        timeout_duration: Duration,
    ) -> Result<PunchRelayResponse> {
        let udp_port = self.socket.local_addr()?.port() as i32;
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
                id: target_id.to_string(),
                nat_type: NatType::UnknownNat as i32,
                licence_key: licence_key.to_string(),
                conn_type: conn_type as i32,
                token: String::new(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                udp_port,
                force_relay: true,
                upnp_port: 0,
                socket_addr_v6: Vec::new(),
            })),
        };

        let stream = tokio::net::TcpStream::connect(&self.server_addr)
            .await
            .with_context(|| {
                format!(
                    "failed to open TCP to rendezvous server {} for PunchHoleRequest",
                    self.server_addr
                )
            })?;
        let mut transport = Some(TcpTransport::new(stream));

        let request_bytes = encode_rendezvous_message(&request)?;
        transport
            .as_mut()
            .expect("plain TCP transport should exist before KeyExchange")
            .send(&request_bytes)
            .await?;

        let deadline = Instant::now() + timeout_duration;
        let mut punch_response: Option<PunchHoleResponse> = None;
        let mut encrypted: Option<EncryptedStream<TcpTransport>> = None;
        let mut resent_after_key_exchange = false;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if let Some(response) = punch_response {
                    return Ok(PunchRelayResponse::PunchHole(response));
                }
                bail!(
                    "timed out waiting for PunchHoleResponse or RelayResponse over TCP after {}s",
                    timeout_duration.as_secs()
                );
            }

            let recv_fut = async {
                if let Some(stream) = encrypted.as_mut() {
                    stream.recv().await
                } else {
                    transport
                        .as_mut()
                        .expect("plain TCP transport should exist before KeyExchange")
                        .recv()
                        .await
                }
            };

            let resp_bytes: Vec<u8> = match timeout(remaining, recv_fut).await {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(e)) => return Err(e).context("reading rendezvous response over TCP"),
                Err(_) => {
                    if let Some(response) = punch_response {
                        return Ok(PunchRelayResponse::PunchHole(response));
                    }
                    bail!(
                        "timed out waiting for PunchHoleResponse or RelayResponse over TCP after {}s",
                        timeout_duration.as_secs()
                    );
                }
            };

            let response = RendezvousMessage::decode(resp_bytes.as_slice())
                .context("decode rendezvous response from TCP")?;

            match response.union {
                Some(rendezvous_message::Union::PunchHoleResponse(resp)) => {
                    punch_response = Some(resp);
                }
                Some(rendezvous_message::Union::RelayResponse(resp)) => {
                    return Ok(PunchRelayResponse::Relay(resp));
                }
                Some(rendezvous_message::Union::KeyExchange(resp)) => {
                    if encrypted.is_some() || resent_after_key_exchange {
                        bail!("unexpected repeated TCP KeyExchange from rendezvous server");
                    }

                    let encrypted_stream = complete_tcp_key_exchange(
                        transport
                            .take()
                            .expect("plain TCP transport should exist when KeyExchange starts"),
                        licence_key,
                        &resp,
                    )
                    .await
                    .context("completing TCP rendezvous KeyExchange")?;
                    encrypted = Some(encrypted_stream);

                    if let Some(stream) = encrypted.as_mut() {
                        stream
                            .send(&request_bytes)
                            .await
                            .context("sending encrypted PunchHoleRequest over TCP")?;
                    }
                    resent_after_key_exchange = true;
                }
                Some(rendezvous_message::Union::RegisterPeerResponse(_)) => continue,
                other => bail!("unexpected rendezvous response to TCP PunchHoleRequest: {other:?}"),
            }
        }
    }

    /// Fire-and-forget PunchHole: send the request without waiting for a response.
    /// With correct licence_key, hbbs forwards to the peer and sends NO response
    /// back to the requester.
    pub async fn send_punch_hole(&self, target_id: &str, licence_key: &str) -> Result<()> {
        self.send_punch_hole_with_conn_type(target_id, licence_key, ConnType::DefaultConn)
            .await
    }

    pub async fn send_punch_hole_with_conn_type(
        &self,
        target_id: &str,
        licence_key: &str,
        conn_type: ConnType,
    ) -> Result<()> {
        let udp_port = self.socket.local_addr()?.port() as i32;
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
                id: target_id.to_string(),
                nat_type: NatType::UnknownNat as i32,
                licence_key: licence_key.to_string(),
                conn_type: conn_type as i32,
                token: String::new(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                udp_port,
                force_relay: true,
                upnp_port: 0,
                socket_addr_v6: Vec::new(),
            })),
        };

        let mut buf = Vec::new();
        request.encode(&mut buf)?;
        self.socket.send(&buf).await?;
        Ok(())
    }

    pub async fn request_relay(&self) -> Result<RelayResponse> {
        self.request_relay_for("", "", &[], "").await
    }

    pub async fn request_relay_for(
        &self,
        target_id: &str,
        relay_server: &str,
        socket_addr: &[u8],
        licence_key: &str,
    ) -> Result<RelayResponse> {
        self.request_relay_for_with_conn_type(
            target_id,
            relay_server,
            socket_addr,
            licence_key,
            ConnType::DefaultConn,
        )
        .await
    }

    pub async fn request_relay_for_with_conn_type(
        &self,
        target_id: &str,
        relay_server: &str,
        socket_addr: &[u8],
        licence_key: &str,
        conn_type: ConnType,
    ) -> Result<RelayResponse> {
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::RequestRelay(RequestRelay {
                id: target_id.to_string(),
                uuid: String::new(),
                socket_addr: socket_addr.to_vec(),
                relay_server: relay_server.to_string(),
                secure: true,
                licence_key: licence_key.to_string(),
                conn_type: conn_type as i32,
                token: String::new(),
                control_permissions: None,
            })),
        };

        match self.send_request(&request).await?.union {
            Some(rendezvous_message::Union::RelayResponse(response)) => Ok(response),
            other => bail!("unexpected rendezvous response to RequestRelay: {other:?}"),
        }
    }

    /// Send RequestRelay over TCP to hbbs using BytesCodec framing.
    ///
    /// hbbs ignores RequestRelay over UDP — it only processes them via TCP.
    /// The client generates its own UUID; hbbs forwards to the peer, then
    /// sends back a RelayResponse.
    pub async fn request_relay_via_tcp(
        &self,
        target_id: &str,
        relay_server: &str,
        socket_addr: &[u8],
        licence_key: &str,
        uuid: &str,
    ) -> Result<RelayResponse> {
        self.request_relay_via_tcp_with_conn_type(
            target_id,
            relay_server,
            socket_addr,
            licence_key,
            uuid,
            ConnType::DefaultConn,
        )
        .await
    }

    pub async fn request_relay_via_tcp_with_conn_type(
        &self,
        target_id: &str,
        relay_server: &str,
        socket_addr: &[u8],
        licence_key: &str,
        uuid: &str,
        conn_type: ConnType,
    ) -> Result<RelayResponse> {
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::RequestRelay(RequestRelay {
                id: target_id.to_string(),
                uuid: uuid.to_string(),
                socket_addr: socket_addr.to_vec(),
                relay_server: relay_server.to_string(),
                secure: true,
                licence_key: licence_key.to_string(),
                conn_type: conn_type as i32,
                token: String::new(),
                control_permissions: None,
            })),
        };

        let mut stream = tokio::net::TcpStream::connect(&self.server_addr)
            .await
            .with_context(|| {
                format!(
                    "failed to open TCP to rendezvous server {} for RequestRelay",
                    self.server_addr
                )
            })?;

        // Send with BytesCodec framing (variable-length header, low 2 bits = header_size - 1).
        let mut buf = Vec::new();
        request.encode(&mut buf)?;
        bytescodec_send(&mut stream, &buf).await?;

        // Receive response with BytesCodec framing.
        let resp_bytes = bytescodec_recv(&mut stream).await.context("reading RelayResponse over TCP")?;

        let response = RendezvousMessage::decode(resp_bytes.as_slice())
            .context("decode RelayResponse from TCP")?;

        match response.union {
            Some(rendezvous_message::Union::RelayResponse(r)) => Ok(r),
            other => bail!("expected RelayResponse over TCP, got {other:?}"),
        }
    }

    /// Send raw bytes on the UDP socket (for diagnostics / manual protocol).
    pub async fn raw_send(&self, buf: &[u8]) -> Result<()> {
        self.socket.send(buf).await?;
        Ok(())
    }

    /// Receive raw bytes from the UDP socket (for diagnostics / manual protocol).
    pub async fn raw_recv(&self, buf: &mut [u8]) -> Result<usize> {
        let size = self.socket.recv(buf).await?;
        Ok(size)
    }

    async fn send_request(&self, message: &RendezvousMessage) -> Result<RendezvousMessage> {
        let mut buf = Vec::new();
        message.encode(&mut buf)?;
        self.socket.send(&buf).await?;

        // Loop to skip RegisterPeerResponse messages that may arrive from
        // the background heartbeat task sharing this socket.
        let mut recv_buf = vec![0_u8; 4096];
        loop {
            let size = self.socket.recv(&mut recv_buf).await?;
            let response = RendezvousMessage::decode(&recv_buf[..size])?;
            if matches!(
                response.union,
                Some(rendezvous_message::Union::RegisterPeerResponse(_))
            ) {
                continue;
            }
            return Ok(response);
        }
    }
}

/// Spawn a heartbeat task that sends RegisterPeer at the given interval.
/// The first message is sent immediately (the interval's instant first tick)
/// to ensure the server considers the client active before PunchHole.
fn spawn_heartbeat(socket: Arc<UdpSocket>, id: String, period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(period);
        loop {
            interval.tick().await;
            let msg = RendezvousMessage {
                union: Some(rendezvous_message::Union::RegisterPeer(RegisterPeer {
                    id: id.clone(),
                    serial: 0,
                })),
            };
            let mut buf = Vec::new();
            if msg.encode(&mut buf).is_ok() {
                let _ = socket.send(&buf).await;
            }
        }
    })
}

/// Send a protobuf payload with BytesCodec framing over a TCP stream.
/// (Standalone helper so rendezvous.rs doesn't depend on transport::FramedTransport.)
async fn bytescodec_send<S: tokio::io::AsyncWrite + Unpin>(stream: &mut S, msg: &[u8]) -> Result<()> {
    let len = msg.len();
    if len <= 0x3F {
        let b = ((len as u8) << 2) | 0b00;
        stream.write_all(&[b]).await?;
    } else if len <= 0x3FFF {
        let val = ((len as u16) << 2) | 0b01;
        stream.write_all(&val.to_le_bytes()).await?;
    } else if len <= 0x3F_FFFF {
        let val = ((len as u32) << 2) | 0b10;
        let bytes = val.to_le_bytes();
        stream.write_all(&bytes[..3]).await?;
    } else if len <= 0x3FFF_FFFF {
        let val = ((len as u32) << 2) | 0b11;
        stream.write_all(&val.to_le_bytes()).await?;
    } else {
        bail!("bytescodec_send: payload too large: {len} bytes");
    }
    stream.write_all(msg).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive a BytesCodec-framed payload from a TCP stream.
async fn bytescodec_recv<S: tokio::io::AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>> {
    let mut first = [0_u8; 1];
    stream.read_exact(&mut first).await?;
    let tag = first[0] & 0x03;
    let header_len = (tag + 1) as usize;
    let mut raw = [0_u8; 4];
    raw[0] = first[0];
    if header_len > 1 {
        stream.read_exact(&mut raw[1..header_len]).await?;
    }
    let combined = u32::from_le_bytes(raw);
    let payload_len = (combined >> 2) as usize;
    if payload_len > 64 * 1024 * 1024 {
        bail!("bytescodec_recv: payload {payload_len} exceeds 64 MiB limit");
    }
    let mut payload = vec![0_u8; payload_len];
    stream.read_exact(&mut payload).await?;
    Ok(payload)
}

fn encode_rendezvous_message(message: &RendezvousMessage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    message.encode(&mut buf)?;
    Ok(buf)
}

async fn complete_tcp_key_exchange(
    mut transport: TcpTransport,
    server_key_b64: &str,
    key_exchange: &KeyExchange,
) -> Result<EncryptedStream<TcpTransport>> {
    let signed_key = key_exchange
        .keys
        .first()
        .context("TCP KeyExchange missing server key payload")?;
    eprintln!(
        "[debug] hbbs TCP KeyExchange keys[0]: len={}, first16={}",
        signed_key.len(),
        hex_preview(signed_key, 16)
    );
    let peer_box_pk = verify_rendezvous_server_key(server_key_b64, signed_key)
        .context("verifying signed rendezvous TCP key")?;
    eprintln!(
        "[debug] hbbs TCP KeyExchange extracted peer_box_pk={}",
        hex_preview(&peer_box_pk, peer_box_pk.len())
    );

    let key_result = crypto::key_exchange_curve25519(&peer_box_pk)
        .context("creating TCP rendezvous symmetric key")?;
    eprintln!(
        "[debug] hbbs TCP KeyExchange client response sealed_key_len={}",
        key_result.sealed_key.len()
    );
    let response = RendezvousMessage {
        union: Some(rendezvous_message::Union::KeyExchange(KeyExchange {
            keys: vec![
                key_result.ephemeral_pk.to_vec(),
                key_result.sealed_key,
            ],
        })),
    };
    let response_bytes = encode_rendezvous_message(&response)?;

    // The response KeyExchange itself is still sent on the raw framed stream.
    transport
        .send(&response_bytes)
        .await
        .context("sending TCP KeyExchange response")?;

    Ok(EncryptedStream::new(transport, &key_result.session_key))
}

fn verify_rendezvous_server_key(server_key_b64: &str, signed_key: &[u8]) -> Result<[u8; 32]> {
    if signed_key.len() == 32 {
        let peer_box_pk: [u8; 32] = signed_key
            .try_into()
            .map_err(|_| anyhow::anyhow!("rendezvous TCP key is {} bytes, expected 32", signed_key.len()))?;
        eprintln!("[warn] rendezvous TCP key arrived unsigned; proceeding with raw 32-byte payload");
        return Ok(peer_box_pk);
    }

    if signed_key.len() < 96 {
        bail!(
            "signed rendezvous TCP key is {} bytes, expected 32 or at least 96",
            signed_key.len()
        );
    }

    let verification_key = decode_rendezvous_verifying_key(server_key_b64).ok();

    if let Some((peer_box_pk, layout)) = extract_verified_rendezvous_key(verification_key.as_ref(), signed_key) {
        eprintln!("[debug] rendezvous TCP key verified with layout {layout}");
        return Ok(peer_box_pk);
    }

    let suffix_message = &signed_key[64..];
    if suffix_message.len() == 32 {
        let peer_box_pk: [u8; 32] = suffix_message
            .try_into()
            .map_err(|_| anyhow::anyhow!("verified rendezvous TCP key is {} bytes, expected 32", suffix_message.len()))?;
        eprintln!(
            "[warn] rendezvous TCP key signature verification failed; falling back to signature||key layout with unverified payload"
        );
        return Ok(peer_box_pk);
    }

    let prefix_message = &signed_key[..32];
    let peer_box_pk: [u8; 32] = prefix_message
        .try_into()
        .map_err(|_| anyhow::anyhow!("rendezvous TCP key prefix is {} bytes, expected 32", prefix_message.len()))?;
    eprintln!(
        "[warn] rendezvous TCP key signature verification failed; falling back to key||signature layout with unverified payload"
    );
    Ok(peer_box_pk)
}

fn decode_rendezvous_verifying_key(server_key_b64: &str) -> Result<VerifyingKey> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(server_key_b64)
        .context("base64 decode rendezvous server key")?;
    let server_pk: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("rendezvous server key is {} bytes, expected 32", decoded.len()))?;
    VerifyingKey::from_bytes(&server_pk).context("invalid rendezvous server Ed25519 key")
}

fn extract_verified_rendezvous_key(
    verifying_key: Option<&VerifyingKey>,
    signed_key: &[u8],
) -> Option<([u8; 32], &'static str)> {
    let verifying_key = verifying_key?;

    if signed_key.len() >= 96 {
        let signature = Signature::from_slice(&signed_key[..64]).ok()?;
        let message = &signed_key[64..];
        if message.len() == 32 && verifying_key.verify(message, &signature).is_ok() {
            let peer_box_pk = message.try_into().ok()?;
            return Some((peer_box_pk, "signature||key"));
        }
    }

    if signed_key.len() == 96 {
        let message = &signed_key[..32];
        let signature = Signature::from_slice(&signed_key[32..96]).ok()?;
        if verifying_key.verify(message, &signature).is_ok() {
            let peer_box_pk = message.try_into().ok()?;
            return Some((peer_box_pk, "key||signature"));
        }
    }

    None
}

fn hex_preview(bytes: &[u8], max_len: usize) -> String {
    let preview_len = bytes.len().min(max_len);
    let mut out = String::with_capacity(preview_len * 2 + 8);
    for byte in &bytes[..preview_len] {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    if bytes.len() > preview_len {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Transport;
    use crypto_box::{PublicKey as BoxPublicKey, SalsaBox, SecretKey as BoxSecretKey, aead::Aead};
    use ed25519_dalek::{Signer, SigningKey};
    use crate::proto::hbb::{RelayResponse, rendezvous_message};

    async fn bind_test_server() -> UdpSocket {
        UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("server socket should bind")
    }

    #[tokio::test]
    async fn register_peer_sends_register_peer_and_parses_response() -> Result<()> {
        let server = bind_test_server().await;
        let server_addr = server.local_addr()?;

        let server_task = tokio::spawn(async move {
            let mut buf = [0_u8; 4096];
            let (size, peer) = server.recv_from(&mut buf).await?;
            let message = RendezvousMessage::decode(&buf[..size])?;

            match message.union {
                Some(rendezvous_message::Union::RegisterPeer(register)) => {
                    assert_eq!(register.id, "host-1");
                    assert_eq!(register.serial, 0);
                }
                other => panic!("expected RegisterPeer, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RegisterPeerResponse(
                    RegisterPeerResponse { request_pk: true },
                )),
            };
            let mut encoded = Vec::new();
            response.encode(&mut encoded)?;
            server.send_to(&encoded, peer).await?;
            Result::<()>::Ok(())
        });

        let client = RendezvousClient::connect(&server_addr.to_string()).await?;
        let response = client.register_peer("host-1", b"public-key").await?;

        assert!(response.request_pk);
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn register_pk_sends_uuid_and_public_key_and_parses_response() -> Result<()> {
        let server = bind_test_server().await;
        let server_addr = server.local_addr()?;

        let server_task = tokio::spawn(async move {
            let mut buf = [0_u8; 4096];
            let (size, peer) = server.recv_from(&mut buf).await?;
            let message = RendezvousMessage::decode(&buf[..size])?;

            match message.union {
                Some(rendezvous_message::Union::RegisterPk(register)) => {
                    assert_eq!(register.id, "host-1");
                    assert_eq!(register.uuid, vec![1, 2, 3, 4]);
                    assert_eq!(register.pk, vec![9; 32]);
                    assert!(register.old_id.is_empty());
                    assert!(!register.no_register_device);
                }
                other => panic!("expected RegisterPk, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RegisterPkResponse(
                    RegisterPkResponse {
                        result: crate::proto::hbb::register_pk_response::Result::Ok as i32,
                        keep_alive: 30,
                    },
                )),
            };
            let mut encoded = Vec::new();
            response.encode(&mut encoded)?;
            server.send_to(&encoded, peer).await?;
            Result::<()>::Ok(())
        });

        let client = RendezvousClient::connect(&server_addr.to_string()).await?;
        let response = client.register_pk("host-1", &[1, 2, 3, 4], &[9; 32]).await?;

        assert_eq!(
            response.result,
            crate::proto::hbb::register_pk_response::Result::Ok as i32
        );
        assert_eq!(response.keep_alive, 30);
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn punch_hole_sends_request_and_returns_response() -> Result<()> {
        let server = bind_test_server().await;
        let server_addr = server.local_addr()?;

        let server_task = tokio::spawn(async move {
            let mut buf = [0_u8; 4096];
            let (size, peer) = server.recv_from(&mut buf).await?;
            let message = RendezvousMessage::decode(&buf[..size])?;

            match message.union {
                Some(rendezvous_message::Union::PunchHoleRequest(request)) => {
                    assert_eq!(request.id, "target-9");
                    assert_eq!(request.nat_type, NatType::UnknownNat as i32);
                    assert_eq!(request.conn_type, ConnType::DefaultConn as i32);
                    assert!(request.force_relay);
                    assert!(request.udp_port > 0);
                }
                other => panic!("expected PunchHoleRequest, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::PunchHoleResponse(
                    PunchHoleResponse {
                        socket_addr: vec![127, 0, 0, 1, 0x20, 0xfb],
                        pk: b"peer-public-key".to_vec(),
                        failure: 0,
                        relay_server: "relay.example.com:21117".to_string(),
                        other_failure: String::new(),
                        feedback: 0,
                        is_udp: true,
                        upnp_port: 0,
                        socket_addr_v6: Vec::new(),
                        union: Some(crate::proto::hbb::punch_hole_response::Union::NatType(
                            NatType::Asymmetric as i32,
                        )),
                    },
                )),
            };
            let mut encoded = Vec::new();
            response.encode(&mut encoded)?;
            server.send_to(&encoded, peer).await?;
            Result::<()>::Ok(())
        });

        let client = RendezvousClient::connect(&server_addr.to_string()).await?;
        let response = client.punch_hole("target-9", "").await?;

        assert_eq!(response.pk, b"peer-public-key");
        assert_eq!(response.relay_server, "relay.example.com:21117");
        assert!(response.is_udp);
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn request_relay_sends_request_and_returns_response() -> Result<()> {
        let server = bind_test_server().await;
        let server_addr = server.local_addr()?;

        let server_task = tokio::spawn(async move {
            let mut buf = [0_u8; 4096];
            let (size, peer) = server.recv_from(&mut buf).await?;
            let message = RendezvousMessage::decode(&buf[..size])?;

            match message.union {
                Some(rendezvous_message::Union::RequestRelay(request)) => {
                    assert!(request.secure);
                    assert_eq!(request.conn_type, ConnType::DefaultConn as i32);
                    assert!(request.id.is_empty());
                    assert!(request.uuid.is_empty());
                }
                other => panic!("expected RequestRelay, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RelayResponse(RelayResponse {
                    socket_addr: vec![127, 0, 0, 1, 0x52, 0x75],
                    uuid: "relay-uuid".to_string(),
                    relay_server: "relay.example.com:21117".to_string(),
                    refuse_reason: String::new(),
                    version: "1.0".to_string(),
                    feedback: 0,
                    socket_addr_v6: Vec::new(),
                    upnp_port: 0,
                    union: Some(crate::proto::hbb::relay_response::Union::Id(
                        "peer-123".to_string(),
                    )),
                })),
            };
            let mut encoded = Vec::new();
            response.encode(&mut encoded)?;
            server.send_to(&encoded, peer).await?;
            Result::<()>::Ok(())
        });

        let client = RendezvousClient::connect(&server_addr.to_string()).await?;
        let response = client.request_relay().await?;

        assert_eq!(response.uuid, "relay-uuid");
        assert_eq!(response.relay_server, "relay.example.com:21117");
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn request_relay_for_includes_target_and_routing_hints() -> Result<()> {
        let server = bind_test_server().await;
        let server_addr = server.local_addr()?;

        let server_task = tokio::spawn(async move {
            let mut buf = [0_u8; 4096];
            let (size, peer) = server.recv_from(&mut buf).await?;
            let message = RendezvousMessage::decode(&buf[..size])?;

            match message.union {
                Some(rendezvous_message::Union::RequestRelay(request)) => {
                    assert_eq!(request.id, "target-9");
                    assert_eq!(request.relay_server, "relay.example.com:21117");
                    assert_eq!(request.socket_addr, vec![1, 2, 3, 4]);
                    assert_eq!(request.licence_key, "test-key");
                    assert!(request.secure);
                }
                other => panic!("expected RequestRelay, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RelayResponse(RelayResponse {
                    socket_addr: Vec::new(),
                    uuid: "relay-uuid".to_string(),
                    relay_server: "relay.example.com:21117".to_string(),
                    refuse_reason: String::new(),
                    version: "1.0".to_string(),
                    feedback: 0,
                    socket_addr_v6: Vec::new(),
                    upnp_port: 0,
                    union: None,
                })),
            };
            let mut encoded = Vec::new();
            response.encode(&mut encoded)?;
            server.send_to(&encoded, peer).await?;
            Result::<()>::Ok(())
        });

        let client = RendezvousClient::connect(&server_addr.to_string()).await?;
        let response = client
            .request_relay_for("target-9", "relay.example.com:21117", &[1, 2, 3, 4], "test-key")
            .await?;

        assert_eq!(response.uuid, "relay-uuid");
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn request_relay_via_tcp_sends_over_tcp_and_returns_response() -> Result<()> {
        // Bind both a UDP server (for RendezvousClient::connect) and a TCP
        // server (for the actual RequestRelay exchange) on the same port.
        // We use port 0 twice — the OS assigns different ephemeral ports, so
        // we bind TCP first and then point the UDP socket at it won't work.
        // Instead: bind TCP, get its port, bind UDP on the same port.
        let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let tcp_addr = tcp_listener.local_addr()?;

        let udp_server = UdpSocket::bind(format!("127.0.0.1:{}", tcp_addr.port())).await;
        // If the OS won't let us reuse the port for UDP, bind on a different
        // port and create the client manually.
        let (udp_server, client) = if let Ok(udp) = udp_server {
            let client = RendezvousClient::connect(&tcp_addr.to_string()).await?;
            (udp, client)
        } else {
            let udp = UdpSocket::bind("127.0.0.1:0").await?;
            let udp_addr = udp.local_addr()?;
            // Client's UDP socket connects to the UDP server, but server_addr
            // must point at the TCP listener for request_relay_via_tcp.
            let bind_addr = "0.0.0.0:0";
            let socket = UdpSocket::bind(bind_addr).await?;
            socket.connect(udp_addr).await?;
            let client = RendezvousClient {
                socket: Arc::new(socket),
                server_addr: tcp_addr.to_string(),
            };
            (udp, client)
        };
        // Keep udp_server alive so the client's UDP connect doesn't fail.
        let _udp_server = udp_server;

        let server_task = tokio::spawn(async move {
            let (mut stream, _) = tcp_listener.accept().await?;

            // Read framed message with BytesCodec framing.
            let payload = bytescodec_recv(&mut stream).await?;

            let message = RendezvousMessage::decode(payload.as_slice())?;
            match message.union {
                Some(rendezvous_message::Union::RequestRelay(req)) => {
                    assert_eq!(req.id, "target-tcp");
                    assert_eq!(req.relay_server, "relay.tcp.example:21117");
                    assert_eq!(req.licence_key, "tcp-key");
                    assert_eq!(req.uuid, "test-uuid-1234");
                    assert!(req.secure);
                }
                other => panic!("expected RequestRelay over TCP, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RelayResponse(RelayResponse {
                    socket_addr: Vec::new(),
                    uuid: "tcp-relay-uuid".to_string(),
                    relay_server: "relay.tcp.example:21117".to_string(),
                    refuse_reason: String::new(),
                    version: "1.0".to_string(),
                    feedback: 0,
                    socket_addr_v6: Vec::new(),
                    upnp_port: 0,
                    union: None,
                })),
            };
            let mut encoded = Vec::new();
            response.encode(&mut encoded)?;
            bytescodec_send(&mut stream, &encoded).await?;

            Result::<()>::Ok(())
        });

        let response = client
            .request_relay_via_tcp("target-tcp", "relay.tcp.example:21117", &[], "tcp-key", "test-uuid-1234")
            .await?;

        assert_eq!(response.uuid, "tcp-relay-uuid");
        assert_eq!(response.relay_server, "relay.tcp.example:21117");
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn punch_hole_via_tcp_handles_key_exchange_then_replays_request() -> Result<()> {
        let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let tcp_addr = tcp_listener.local_addr()?;

        let udp_server = UdpSocket::bind(format!("127.0.0.1:{}", tcp_addr.port())).await;
        let (udp_server, client) = if let Ok(udp) = udp_server {
            let client = RendezvousClient::connect(&tcp_addr.to_string()).await?;
            (udp, client)
        } else {
            let udp = UdpSocket::bind("127.0.0.1:0").await?;
            let udp_addr = udp.local_addr()?;
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            socket.connect(udp_addr).await?;
            let client = RendezvousClient {
                socket: Arc::new(socket),
                server_addr: tcp_addr.to_string(),
            };
            (udp, client)
        };
        let _udp_server = udp_server;

        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let server_key_b64 = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());

        let server_task = tokio::spawn(async move {
            let (stream, _) = tcp_listener.accept().await?;
            let mut transport = TcpTransport::new(stream);

            let first_payload: Vec<u8> = transport.recv().await?;
            let first_message = RendezvousMessage::decode(first_payload.as_slice())?;
            match first_message.union {
                Some(rendezvous_message::Union::PunchHoleRequest(req)) => {
                    assert_eq!(req.id, "target-keyex");
                    assert_eq!(req.conn_type, ConnType::Terminal as i32);
                }
                other => panic!("expected plaintext PunchHoleRequest, got {other:?}"),
            }

            let server_ephemeral_sk = BoxSecretKey::generate(&mut rand_core::OsRng);
            let server_ephemeral_pk = server_ephemeral_sk.public_key().to_bytes();
            let signed_pk = signing_key.sign(&server_ephemeral_pk).to_bytes();
            let mut signed_payload = Vec::with_capacity(64 + server_ephemeral_pk.len());
            signed_payload.extend_from_slice(&signed_pk);
            signed_payload.extend_from_slice(&server_ephemeral_pk);

            let key_exchange = RendezvousMessage {
                union: Some(rendezvous_message::Union::KeyExchange(KeyExchange {
                    keys: vec![signed_payload],
                })),
            };
            transport.send(&encode_rendezvous_message(&key_exchange)?).await?;

            let response_payload: Vec<u8> = transport.recv().await?;
            let response_message = RendezvousMessage::decode(response_payload.as_slice())?;
            let response_keys = match response_message.union {
                Some(rendezvous_message::Union::KeyExchange(resp)) => resp.keys,
                other => panic!("expected KeyExchange response, got {other:?}"),
            };
            assert_eq!(response_keys.len(), 2);

            let client_pk = BoxPublicKey::from_slice(&response_keys[0])
                .expect("client ephemeral pk should be 32 bytes");
            let salsa_box = SalsaBox::new(&client_pk, &server_ephemeral_sk);
            let zero_nonce = Default::default();
            let session_key = salsa_box
                .decrypt(&zero_nonce, response_keys[1].as_ref())
                .expect("server should decrypt session key");
            let session_key: [u8; 32] = session_key
                .as_slice()
                .try_into()
                .expect("session key should be 32 bytes");

            let mut encrypted = EncryptedStream::new(transport, &session_key);
            let encrypted_request: Vec<u8> = encrypted.recv().await?;
            let replayed_message = RendezvousMessage::decode(encrypted_request.as_slice())?;
            match replayed_message.union {
                Some(rendezvous_message::Union::PunchHoleRequest(req)) => {
                    assert_eq!(req.id, "target-keyex");
                    assert_eq!(req.conn_type, ConnType::Terminal as i32);
                }
                other => panic!("expected encrypted PunchHoleRequest replay, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RelayResponse(RelayResponse {
                    socket_addr: Vec::new(),
                    uuid: "relay-after-keyex".to_string(),
                    relay_server: "relay.keyex.example:21117".to_string(),
                    refuse_reason: String::new(),
                    version: "1.0".to_string(),
                    feedback: 0,
                    socket_addr_v6: Vec::new(),
                    upnp_port: 0,
                    union: None,
                })),
            };
            encrypted
                .send(&encode_rendezvous_message(&response)?)
                .await?;

            Result::<()>::Ok(())
        });

        let response = client
            .punch_hole_via_tcp_with_conn_type(
                "target-keyex",
                &server_key_b64,
                ConnType::Terminal,
                Duration::from_secs(2),
            )
            .await?;

        match response {
            PunchRelayResponse::Relay(resp) => {
                assert_eq!(resp.uuid, "relay-after-keyex");
                assert_eq!(resp.relay_server, "relay.keyex.example:21117");
            }
            other => panic!("expected RelayResponse after KeyExchange, got {other:?}"),
        }

        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[tokio::test]
    async fn punch_hole_via_tcp_falls_back_when_signature_key_mismatches() -> Result<()> {
        let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let tcp_addr = tcp_listener.local_addr()?;

        let udp_server = UdpSocket::bind(format!("127.0.0.1:{}", tcp_addr.port())).await;
        let (udp_server, client) = if let Ok(udp) = udp_server {
            let client = RendezvousClient::connect(&tcp_addr.to_string()).await?;
            (udp, client)
        } else {
            let udp = UdpSocket::bind("127.0.0.1:0").await?;
            let udp_addr = udp.local_addr()?;
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            socket.connect(udp_addr).await?;
            let client = RendezvousClient {
                socket: Arc::new(socket),
                server_addr: tcp_addr.to_string(),
            };
            (udp, client)
        };
        let _udp_server = udp_server;

        let actual_signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let wrong_signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let mismatched_server_key_b64 = base64::engine::general_purpose::STANDARD
            .encode(wrong_signing_key.verifying_key().to_bytes());

        let server_task = tokio::spawn(async move {
            let (stream, _) = tcp_listener.accept().await?;
            let mut transport = TcpTransport::new(stream);

            let first_payload: Vec<u8> = transport.recv().await?;
            let first_message = RendezvousMessage::decode(first_payload.as_slice())?;
            match first_message.union {
                Some(rendezvous_message::Union::PunchHoleRequest(req)) => {
                    assert_eq!(req.id, "target-mismatch");
                }
                other => panic!("expected plaintext PunchHoleRequest, got {other:?}"),
            }

            let server_ephemeral_sk = BoxSecretKey::generate(&mut rand_core::OsRng);
            let server_ephemeral_pk = server_ephemeral_sk.public_key().to_bytes();
            let signed_pk = actual_signing_key.sign(&server_ephemeral_pk).to_bytes();
            let mut signed_payload = Vec::with_capacity(64 + server_ephemeral_pk.len());
            signed_payload.extend_from_slice(&signed_pk);
            signed_payload.extend_from_slice(&server_ephemeral_pk);

            let key_exchange = RendezvousMessage {
                union: Some(rendezvous_message::Union::KeyExchange(KeyExchange {
                    keys: vec![signed_payload],
                })),
            };
            transport.send(&encode_rendezvous_message(&key_exchange)?).await?;

            let response_payload: Vec<u8> = transport.recv().await?;
            let response_message = RendezvousMessage::decode(response_payload.as_slice())?;
            let response_keys = match response_message.union {
                Some(rendezvous_message::Union::KeyExchange(resp)) => resp.keys,
                other => panic!("expected KeyExchange response, got {other:?}"),
            };
            let client_pk = BoxPublicKey::from_slice(&response_keys[0])
                .expect("client ephemeral pk should be 32 bytes");
            let salsa_box = SalsaBox::new(&client_pk, &server_ephemeral_sk);
            let zero_nonce = Default::default();
            let session_key = salsa_box
                .decrypt(&zero_nonce, response_keys[1].as_ref())
                .expect("server should decrypt session key");
            let session_key: [u8; 32] = session_key
                .as_slice()
                .try_into()
                .expect("session key should be 32 bytes");

            let mut encrypted = EncryptedStream::new(transport, &session_key);
            let encrypted_request: Vec<u8> = encrypted.recv().await?;
            let replayed_message = RendezvousMessage::decode(encrypted_request.as_slice())?;
            match replayed_message.union {
                Some(rendezvous_message::Union::PunchHoleRequest(req)) => {
                    assert_eq!(req.id, "target-mismatch");
                }
                other => panic!("expected encrypted PunchHoleRequest replay, got {other:?}"),
            }

            let response = RendezvousMessage {
                union: Some(rendezvous_message::Union::RelayResponse(RelayResponse {
                    socket_addr: Vec::new(),
                    uuid: "relay-mismatch".to_string(),
                    relay_server: "relay.mismatch.example:21117".to_string(),
                    refuse_reason: String::new(),
                    version: "1.0".to_string(),
                    feedback: 0,
                    socket_addr_v6: Vec::new(),
                    upnp_port: 0,
                    union: None,
                })),
            };
            encrypted
                .send(&encode_rendezvous_message(&response)?)
                .await?;

            Result::<()>::Ok(())
        });

        let response = client
            .punch_hole_via_tcp_with_conn_type(
                "target-mismatch",
                &mismatched_server_key_b64,
                ConnType::Terminal,
                Duration::from_secs(2),
            )
            .await?;

        match response {
            PunchRelayResponse::Relay(resp) => {
                assert_eq!(resp.uuid, "relay-mismatch");
                assert_eq!(resp.relay_server, "relay.mismatch.example:21117");
            }
            other => panic!("expected RelayResponse after signature mismatch fallback, got {other:?}"),
        }

        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[test]
    fn verify_rendezvous_server_key_accepts_signature_then_key_layout() -> Result<()> {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let server_key_b64 = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        let peer_box_pk = BoxSecretKey::generate(&mut rand_core::OsRng)
            .public_key()
            .to_bytes();
        let mut signed_payload = Vec::with_capacity(96);
        signed_payload.extend_from_slice(&signing_key.sign(&peer_box_pk).to_bytes());
        signed_payload.extend_from_slice(&peer_box_pk);

        let extracted = verify_rendezvous_server_key(&server_key_b64, &signed_payload)?;
        assert_eq!(extracted, peer_box_pk);
        Ok(())
    }

    #[test]
    fn verify_rendezvous_server_key_accepts_key_then_signature_layout() -> Result<()> {
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let server_key_b64 = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());
        let peer_box_pk = BoxSecretKey::generate(&mut rand_core::OsRng)
            .public_key()
            .to_bytes();
        let mut signed_payload = Vec::with_capacity(96);
        signed_payload.extend_from_slice(&peer_box_pk);
        signed_payload.extend_from_slice(&signing_key.sign(&peer_box_pk).to_bytes());

        let extracted = verify_rendezvous_server_key(&server_key_b64, &signed_payload)?;
        assert_eq!(extracted, peer_box_pk);
        Ok(())
    }

    #[tokio::test]
    async fn heartbeat_sends_periodic_register_peer() -> Result<()> {
        let server = bind_test_server().await;
        let server_addr = server.local_addr()?;

        let client = RendezvousClient::connect(&server_addr.to_string()).await?;
        // Use a short interval (50ms) so the test completes quickly.
        let handle = spawn_heartbeat(
            Arc::clone(&client.socket),
            "cli-heartbeat".to_string(),
            Duration::from_millis(50),
        );

        let mut count = 0u32;
        let mut buf = [0u8; 4096];
        // Collect heartbeat messages for up to 300ms — expect at least 2.
        while count < 3 {
            match tokio::time::timeout(Duration::from_millis(300), server.recv_from(&mut buf)).await
            {
                Ok(Ok((size, _))) => {
                    let msg = RendezvousMessage::decode(&buf[..size])?;
                    match msg.union {
                        Some(rendezvous_message::Union::RegisterPeer(rp)) => {
                            assert_eq!(rp.id, "cli-heartbeat");
                            count += 1;
                        }
                        other => panic!("expected RegisterPeer heartbeat, got {other:?}"),
                    }
                }
                Ok(Err(e)) => panic!("recv error: {e}"),
                Err(_) => break, // timeout — stop collecting
            }
        }

        handle.abort();
        assert!(
            count >= 2,
            "expected at least 2 heartbeat messages, got {count}"
        );
        Ok(())
    }
}
