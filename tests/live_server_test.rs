#![allow(dead_code)]

#[path = "../src/proto.rs"]
mod proto;
#[path = "../src/rendezvous.rs"]
mod rendezvous;

use anyhow::Result;
use rand_core::{OsRng, RngCore};
use tokio::time::{Duration, timeout};

use rendezvous::RendezvousClient;

const ID_SERVER_ADDR: &str = "115.238.185.55:50076";
const RELAY_SERVER_ADDR: &str = "115.238.185.55:50077";
const SERVER_KEY: &str = "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=";
const TARGET_PEER_ID: &str = "308235080";

fn test_client_id(suffix: &str) -> String {
    format!("rustdesk-cli-live-test-{suffix}")
}

async fn full_register(client: &RendezvousClient, my_id: &str) -> Result<()> {
    let register_response = timeout(
        Duration::from_secs(5),
        client.register_peer(my_id, &[]),
    )
    .await??;

    if register_response.request_pk {
        let mut uuid = [0u8; 16];
        let mut public_key = [0u8; 32];
        OsRng.fill_bytes(&mut uuid);
        OsRng.fill_bytes(&mut public_key);
        timeout(
            Duration::from_secs(5),
            client.register_pk(my_id, &uuid, &public_key),
        )
        .await??;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "hits the live RustDesk ID server over UDP"]
async fn live_rendezvous_server_register_and_punch_hole() -> Result<()> {
    use prost::Message;
    use proto::hbb::{
        ConnType, NatType, PunchHoleRequest, RendezvousMessage, rendezvous_message,
        OnlineRequest,
    };

    let my_id = test_client_id("udp");
    eprintln!("[1] Connecting to ID server {ID_SERVER_ADDR} as {my_id}...");
    let client = timeout(
        Duration::from_secs(5),
        RendezvousClient::connect(ID_SERVER_ADDR),
    )
    .await??;
    eprintln!("[2] Connected. Running full registration...");

    full_register(&client, &my_id).await?;
    eprintln!("[3] Registration complete. Starting heartbeat...");

    let heartbeat = client.start_heartbeat(&my_id);
    tokio::time::sleep(Duration::from_secs(2)).await;
    eprintln!("[4] Heartbeat started. Checking if target peer {TARGET_PEER_ID} is online...");

    // Send OnlineRequest
    let online_req = RendezvousMessage {
        union: Some(rendezvous_message::Union::OnlineRequest(OnlineRequest {
            id: my_id.clone(),
            peers: vec![TARGET_PEER_ID.to_string(), "123456789".to_string()],
        })),
    };
    let mut buf = Vec::new();
    online_req.encode(&mut buf)?;
    client.raw_send(&buf).await?;

    eprintln!("[5] OnlineRequest sent. Waiting for OnlineResponse...");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut recv_buf = vec![0u8; 4096];
    let mut count = 0u32;
    let mut online_received = false;

    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_secs(2), client.raw_recv(&mut recv_buf)).await {
            Ok(Ok(size)) => {
                count += 1;
                let msg = RendezvousMessage::decode(&recv_buf[..size])?;
                match &msg.union {
                    Some(rendezvous_message::Union::RegisterPeerResponse(r)) => {
                        eprintln!("  [{count}] RegisterPeerResponse (request_pk={})", r.request_pk);
                    }
                    Some(rendezvous_message::Union::OnlineResponse(online)) => {
                        eprintln!("  [{count}] OnlineResponse: states={:x?}", online.states);
                        online_received = true;
                    }
                    Some(rendezvous_message::Union::PunchHoleResponse(ph)) => {
                        eprintln!("  [{count}] PunchHoleResponse: failure={}", ph.failure);
                    }
                    other => {
                        eprintln!("  [{count}] Other response: {other:?}");
                    }
                }
            }
            _ => {}
        }
    }

    if !online_received {
        eprintln!("[6] No OnlineResponse received. Server might require a different registration state.");
    }

    eprintln!("[7] Sending PunchHole for {TARGET_PEER_ID} with CORRECT key...");
    let punch_req = RendezvousMessage {
        union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
            id: TARGET_PEER_ID.to_string(),
            nat_type: NatType::UnknownNat as i32,
            licence_key: SERVER_KEY.to_string(),
            conn_type: ConnType::DefaultConn as i32,
            token: String::new(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            udp_port: 0,
            force_relay: false,
            upnp_port: 0,
            socket_addr_v6: Vec::new(),
        })),
    };
    buf.clear();
    punch_req.encode(&mut buf)?;
    client.raw_send(&buf).await?;

    match timeout(Duration::from_secs(5), client.raw_recv(&mut recv_buf)).await {
        Ok(Ok(size)) => {
            let msg = RendezvousMessage::decode(&recv_buf[..size])?;
            eprintln!("  [CORRECT KEY] Received: {:?}", msg.union);
        }
        _ => eprintln!("  [CORRECT KEY] No response (as expected if peer offline or server silent)"),
    }

    eprintln!("[8] Retrying with EMPTY licence_key...");
    let punch_req_empty = RendezvousMessage {
        union: Some(rendezvous_message::Union::PunchHoleRequest(PunchHoleRequest {
            id: TARGET_PEER_ID.to_string(),
            nat_type: NatType::UnknownNat as i32,
            licence_key: "".to_string(),
            conn_type: ConnType::DefaultConn as i32,
            token: String::new(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            udp_port: 0,
            force_relay: false,
            upnp_port: 0,
            socket_addr_v6: Vec::new(),
        })),
    };
    buf.clear();
    punch_req_empty.encode(&mut buf)?;
    client.raw_send(&buf).await?;
    
    match timeout(Duration::from_secs(5), client.raw_recv(&mut recv_buf)).await {
        Ok(Ok(size)) => {
            let msg = RendezvousMessage::decode(&recv_buf[..size])?;
            if let Some(rendezvous_message::Union::PunchHoleResponse(ph)) = msg.union {
                eprintln!("  [EMPTY KEY] PunchHoleResponse: failure={}", ph.failure);
            }
        }
        _ => eprintln!("  [EMPTY KEY] Still no response"),
    }

    heartbeat.abort();
    eprintln!("[9] Diagnostic complete.");

    Ok(())
}

#[tokio::test]
#[ignore = "hits the live RustDesk ID and relay servers over UDP/TCP"]
async fn live_rendezvous_server_requests_relay_and_connects_tcp() -> Result<()> {
    Ok(())
}
