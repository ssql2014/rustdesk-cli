//! Rendezvous client for RustDesk's hbbs server.

use anyhow::{Context, Result, bail};
use prost::Message;
use tokio::net::UdpSocket;

use crate::proto::hbb::{
    ConnType, NatType, PunchHoleRequest, PunchHoleResponse, RegisterPeer, RegisterPeerResponse,
    RegisterPk, RegisterPkResponse, RelayResponse, RendezvousMessage, RequestRelay,
    rendezvous_message,
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
        let udp_port = self.socket.local_addr()?.port() as i32;
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
                id: target_id.to_string(),
                nat_type: NatType::UnknownNat as i32,
                licence_key: licence_key.to_string(),
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
        self.request_relay_for("", "", &[]).await
    }

    pub async fn request_relay_for(
        &self,
        target_id: &str,
        relay_server: &str,
        socket_addr: &[u8],
    ) -> Result<RelayResponse> {
        let request = RendezvousMessage {
            union: Some(rendezvous_message::Union::RequestRelay(RequestRelay {
                id: target_id.to_string(),
                uuid: String::new(),
                socket_addr: socket_addr.to_vec(),
                relay_server: relay_server.to_string(),
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
            .request_relay_for("target-9", "relay.example.com:21117", &[1, 2, 3, 4])
            .await?;

        assert_eq!(response.uuid, "relay-uuid");
        server_task.await.expect("server task should join")?;
        Ok(())
    }
}
