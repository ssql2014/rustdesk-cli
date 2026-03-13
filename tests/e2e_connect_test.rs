#![allow(dead_code)]

#[path = "../src/proto.rs"]
mod proto;
#[path = "../src/rendezvous.rs"]
mod rendezvous;
#[path = "../src/transport.rs"]
mod transport;
#[path = "../src/crypto.rs"]
mod crypto;

use anyhow::{Context, Result, bail};
use prost::Message as ProstMessage;
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

use crypto::{EncryptedStream, key_exchange, password_hash};
use proto::hbb::{
    Hash, LoginRequest, Message, OptionMessage, PublicKey, RendezvousMessage, RequestRelay,
    message, rendezvous_message,
};
use rendezvous::RendezvousClient;
use transport::{TcpTransport, Transport};

const ID_SERVER_ADDR: &str = "115.238.185.55:50076";
const RELAY_SERVER_ADDR: &str = "115.238.185.55:50077";
const TARGET_PEER_ID: &str = "308235080";
const TARGET_PASSWORD: &str = "Evas@2026";
const CLIENT_ID: &str = "rustdesk-cli-e2e-auth";
const CLIENT_NAME: &str = "rustdesk-cli-e2e";
const SERVER_KEY: &str = "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=";
const SERVER_ED25519_PK: [u8; 32] = [
    73, 103, 52, 52, 133, 133, 211, 4, 123, 145, 223, 43, 29, 209, 141, 104, 33, 215, 182, 158,
    221, 138, 181, 8, 152, 75, 107, 86, 100, 95, 65, 215,
];

#[tokio::test]
#[ignore = "hits the live RustDesk ID and relay servers and probes the auth flow"]
async fn live_e2e_connect_auth_probe() -> Result<()> {
    let rendezvous = timeout(
        Duration::from_secs(5),
        RendezvousClient::connect(ID_SERVER_ADDR),
    )
    .await??;

    timeout(
        Duration::from_secs(5),
        rendezvous.register_peer(CLIENT_ID, &SERVER_ED25519_PK),
    )
    .await??;

    let punch_hole_response =
        timeout(Duration::from_secs(5), rendezvous.punch_hole(TARGET_PEER_ID, SERVER_KEY)).await??;

    let relay_server_hint = if punch_hole_response.relay_server.is_empty() {
        RELAY_SERVER_ADDR
    } else {
        punch_hole_response.relay_server.as_str()
    };
    let relay_response = timeout(
        Duration::from_secs(10),
        rendezvous.request_relay_for(
            TARGET_PEER_ID,
            relay_server_hint,
            &punch_hole_response.socket_addr,
            SERVER_KEY,
        ),
    )
    .await;

    let (relay_addr, relay_uuid) = match relay_response {
        Ok(Ok(response)) => (
            if response.relay_server.is_empty() {
                RELAY_SERVER_ADDR.to_string()
            } else {
                response.relay_server
            },
            response.uuid,
        ),
        Ok(Err(err)) => return Err(err).context("RequestRelay returned an error"),
        Err(_) => (RELAY_SERVER_ADDR.to_string(), String::new()),
    };

    let stream = timeout(Duration::from_secs(10), TcpStream::connect(&relay_addr))
        .await
        .with_context(|| format!("timed out connecting to relay {relay_addr}"))??;
    let mut transport = TcpTransport::new(stream);

    let bind_request = RendezvousMessage {
        union: Some(rendezvous_message::Union::RequestRelay(RequestRelay {
            id: TARGET_PEER_ID.to_string(),
            uuid: relay_uuid,
            socket_addr: punch_hole_response.socket_addr.clone(),
            relay_server: relay_addr.clone(),
            secure: true,
            licence_key: String::new(),
            conn_type: proto::hbb::ConnType::DefaultConn as i32,
            token: String::new(),
            control_permissions: None,
        })),
    };
    let mut bind_bytes = Vec::new();
    bind_request.encode(&mut bind_bytes)?;
    timeout(Duration::from_secs(5), transport.send(&bind_bytes)).await??;

    let first_bytes = timeout(Duration::from_secs(10), transport.recv())
        .await
        .context("timed out waiting for first session message after relay bind")?
        .context("relay server closed before forwarding the first session message")?;
    let first_msg = Message::decode(first_bytes.as_slice())
        .context("failed to decode first post-bind Message")?;

    match &first_msg.union {
        Some(message::Union::SignedId(_)) => {}
        other => bail!("unexpected first post-bind message: {other:?}"),
    }

    let kx = key_exchange(&SERVER_ED25519_PK)?;
    let public_key_msg = Message {
        union: Some(message::Union::PublicKey(PublicKey {
            asymmetric_value: kx.ephemeral_pk.to_vec(),
            symmetric_value: kx.sealed_key,
        })),
    };
    let mut public_key_bytes = Vec::new();
    public_key_msg.encode(&mut public_key_bytes)?;
    timeout(Duration::from_secs(5), transport.send(&public_key_bytes)).await??;

    let mut encrypted = EncryptedStream::new(transport, &kx.session_key);
    let hash_bytes = timeout(Duration::from_secs(10), encrypted.recv())
        .await
        .context("timed out waiting for encrypted Hash challenge")?
        .context("encrypted stream closed before Hash challenge")?;
    let hash_msg = Message::decode(hash_bytes.as_slice())
        .context("failed to decode encrypted challenge Message")?;
    let hash = match hash_msg.union {
        Some(message::Union::Hash(hash)) => hash,
        other => bail!("expected encrypted Hash challenge, got {other:?}"),
    };

    let login_request = build_login_request(&hash);
    let login_msg = Message {
        union: Some(message::Union::LoginRequest(login_request)),
    };
    let mut login_bytes = Vec::new();
    login_msg.encode(&mut login_bytes)?;
    timeout(Duration::from_secs(5), encrypted.send(&login_bytes)).await??;

    let response_bytes = timeout(Duration::from_secs(10), encrypted.recv())
        .await
        .context("timed out waiting for encrypted post-login response")?
        .context("encrypted stream closed before post-login response")?;
    let response = Message::decode(response_bytes.as_slice())
        .context("failed to decode encrypted post-login response")?;

    match response.union {
        Some(message::Union::LoginResponse(login_response)) => {
            if let Some(proto::hbb::login_response::Union::PeerInfo(peer_info)) = login_response.union
            {
                assert_eq!(peer_info.username, TARGET_PEER_ID);
                return Ok(());
            }
            panic!("login response: {:?}", login_response);
        }
        other => panic!("post-login message: {other:?}"),
    }
}

fn build_login_request(hash: &Hash) -> LoginRequest {
    let hashed_password = password_hash(
        TARGET_PASSWORD,
        hash.salt.as_bytes(),
        hash.challenge.as_bytes(),
    );

    LoginRequest {
        username: TARGET_PEER_ID.to_string(),
        password: hashed_password.to_vec(),
        my_id: CLIENT_ID.to_string(),
        my_name: CLIENT_NAME.to_string(),
        option: Some(OptionMessage::default()),
        video_ack_required: false,
        session_id: 1,
        version: env!("CARGO_PKG_VERSION").to_string(),
        os_login: None,
        my_platform: std::env::consts::OS.to_string(),
        hwid: Vec::new(),
        avatar: String::new(),
        union: None,
    }
}
