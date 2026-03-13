#![allow(dead_code)]

#[path = "../src/proto.rs"]
mod proto;
#[path = "../src/rendezvous.rs"]
mod rendezvous;

use anyhow::Result;
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

use proto::hbb::punch_hole_response::Failure;
use rendezvous::RendezvousClient;

const ID_SERVER_ADDR: &str = "115.238.185.55:50076";
const RELAY_SERVER_ADDR: &str = "115.238.185.55:50077";
const SERVER_KEY: &str = "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=";
const TARGET_PEER_ID: &str = "308235080";

fn test_client_id(suffix: &str) -> String {
    format!("rustdesk-cli-live-test-{suffix}")
}

#[tokio::test]
#[ignore = "hits the live RustDesk ID server over UDP"]
async fn live_rendezvous_server_register_and_punch_hole() -> Result<()> {
    let client = timeout(
        Duration::from_secs(5),
        RendezvousClient::connect(ID_SERVER_ADDR),
    )
    .await??;

    let register_response = timeout(
        Duration::from_secs(5),
        client.register_peer(&test_client_id("udp"), SERVER_KEY.as_bytes()),
    )
    .await??;

    let _ = register_response;

    let punch_hole_response = timeout(Duration::from_secs(5), client.punch_hole(TARGET_PEER_ID))
        .await??;

    let failure = Failure::try_from(punch_hole_response.failure).ok();
    assert_ne!(
        failure,
        Some(Failure::IdNotExist),
        "expected live target peer {TARGET_PEER_ID} to exist"
    );
    assert!(
        punch_hole_response.failure >= 0,
        "punch hole response should decode: {punch_hole_response:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "hits the live RustDesk ID and relay servers over UDP/TCP"]
async fn live_rendezvous_server_requests_relay_and_connects_tcp() -> Result<()> {
    let client = timeout(
        Duration::from_secs(5),
        RendezvousClient::connect(ID_SERVER_ADDR),
    )
    .await??;

    let register_response = timeout(
        Duration::from_secs(5),
        client.register_peer(&test_client_id("relay"), SERVER_KEY.as_bytes()),
    )
    .await??;

    let _ = register_response;

    let punch_hole_response = timeout(Duration::from_secs(5), client.punch_hole(TARGET_PEER_ID))
        .await??;

    let failure = Failure::try_from(punch_hole_response.failure).ok();
    assert_ne!(
        failure,
        Some(Failure::IdNotExist),
        "expected live target peer {TARGET_PEER_ID} to exist"
    );

    let relay_response = timeout(Duration::from_secs(5), client.request_relay()).await??;
    let relay_addr = if relay_response.relay_server.is_empty() {
        RELAY_SERVER_ADDR.to_string()
    } else {
        relay_response.relay_server.clone()
    };

    let stream = timeout(Duration::from_secs(5), TcpStream::connect(&relay_addr)).await??;
    assert_eq!(
        stream.peer_addr()?.to_string(),
        relay_addr,
        "TCP connection should reach the relay endpoint returned by hbbs"
    );

    Ok(())
}
