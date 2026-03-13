//! Rendezvous client for RustDesk's hbbs server.

use anyhow::{Context, Result, bail};
use prost::Message;
use tokio::net::UdpSocket;

use crate::proto::hbb::{
    ConnType, NatType, PunchHoleRequest, PunchHoleResponse, RegisterPeer, RegisterPeerResponse,
    RelayResponse, RendezvousMessage, RequestRelay, rendezvous_message,
};

pub struct RendezvousClient {
    socket: UdpSocket,
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
        Ok(Self { socket })
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

        match self.send_request(&request).await?.union {
            Some(rendezvous_message::Union::RegisterPeerResponse(response)) => Ok(response),
            other => bail!("unexpected rendezvous response to RegisterPeer: {other:?}"),
        }
    }

    pub async fn punch_hole(&self, target_id: &str) -> Result<PunchHoleResponse> {
        let udp_port = self.socket.local_addr()?.port() as i32;
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
                id: target_id.to_string(),
                nat_type: NatType::UnknownNat as i32,
                licence_key: String::new(),
                conn_type: ConnType::DefaultConn as i32,
                token: String::new(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                udp_port,
                force_relay: false,
                upnp_port: 0,
                socket_addr_v6: Vec::new(),
            })),
        };

        match self.send_request(&request).await?.union {
            Some(rendezvous_message::Union::PunchHoleResponse(response)) => Ok(response),
            other => bail!("unexpected rendezvous response to PunchHoleRequest: {other:?}"),
        }
    }

    pub async fn request_relay(&self) -> Result<RelayResponse> {
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::RequestRelay(RequestRelay {
                id: String::new(),
                uuid: String::new(),
                socket_addr: Vec::new(),
                relay_server: String::new(),
                secure: true,
                licence_key: String::new(),
                conn_type: ConnType::DefaultConn as i32,
                token: String::new(),
                control_permissions: None,
            })),
        };

        match self.send_request(&request).await?.union {
            Some(rendezvous_message::Union::RelayResponse(response)) => Ok(response),
            other => bail!("unexpected rendezvous response to RequestRelay: {other:?}"),
        }
    }

    async fn send_request(&self, message: &RendezvousMessage) -> Result<RendezvousMessage> {
        let mut buf = Vec::new();
        message.encode(&mut buf)?;
        self.socket.send(&buf).await?;

        let mut recv_buf = vec![0_u8; 4096];
        let size = self.socket.recv(&mut recv_buf).await?;
        Ok(RendezvousMessage::decode(&recv_buf[..size])?)
    }
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
                    assert!(!request.force_relay);
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
        let response = client.punch_hole("target-9").await?;

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
}
