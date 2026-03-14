//! Rendezvous client for RustDesk's hbbs server.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use prost::Message;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::proto::hbb::{
    ConnType, NatType, PunchHoleRequest, PunchHoleResponse, RegisterPeer, RegisterPeerResponse,
    RegisterPk, RegisterPkResponse, RelayResponse, RendezvousMessage, RequestRelay,
    rendezvous_message,
};

pub struct RendezvousClient {
    socket: Arc<UdpSocket>,
    server_addr: String,
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
            Some(rendezvous_message::Union::PunchHoleResponse(response)) => Ok(response),
            other => bail!("unexpected rendezvous response to PunchHoleRequest: {other:?}"),
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

#[cfg(test)]
mod tests {
    use super::*;
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
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
