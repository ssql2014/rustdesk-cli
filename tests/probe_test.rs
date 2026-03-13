#[path = "../src/proto.rs"]
mod proto;
#[path = "../src/rendezvous.rs"]
mod rendezvous;

use anyhow::Result;
use proto::hbb::punch_hole_response::Failure;
use rendezvous::RendezvousClient;

#[tokio::test]
async fn probe_peer_status() -> Result<()> {
    let id_server = "115.238.185.55:50076";
    let target_id = "308235080";
    
    println!("Connecting to rendezvous {}...", id_server);
    let client = RendezvousClient::connect(id_server).await?;
    
    println!("Registering...");
    client.register_peer("probe-cli", &[]).await?;
    
    println!("Sending PunchHoleRequest for {}...", target_id);
    let resp = client.punch_hole(target_id).await?;
    
    println!("Response:");
    println!("  failure: {}", resp.failure);
    println!("  is_udp: {}", resp.is_udp);
    println!("  relay_server: {}", resp.relay_server);
    
    let failure_msg = match resp.failure {
        0 => "SUCCESS",
        f if f == Failure::IdNotExist as i32 => "ID_NOT_EXIST",
        f if f == Failure::Offline as i32 => "OFFLINE",
        f if f == Failure::LicenseMismatch as i32 => "LICENSE_MISMATCH",
        _ => "OTHER FAILURE",
    };
    println!("Target peer status: {}", failure_msg);

    println!("Sending RequestRelay...");
    let relay_resp = client.request_relay_for(target_id, "115.238.185.55:50077", &resp.socket_addr).await?;
    println!("Relay Response:");
    println!("  uuid: {}", relay_resp.uuid);
    println!("  relay_server: {}", relay_resp.relay_server);
    println!("  refuse_reason: {}", relay_resp.refuse_reason);
    
    Ok(())
}
