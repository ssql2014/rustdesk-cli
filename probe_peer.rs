#[path = "src/proto.rs"]
mod proto;
#[path = "src/rendezvous.rs"]
mod rendezvous;

use anyhow::Result;
use proto::hbb::punch_hole_response::Failure;
use rendezvous::RendezvousClient;

#[tokio::main]
async fn main() -> Result<()> {
    let id_server = "115.238.185.55:50076";
    let target_id = "308235080";
    
    println!("Connecting to rendezvous {}...", id_server);
    let client = RendezvousClient::connect(id_server).await?;
    
    println!("Registering...");
    client.register_peer("probe-cli", &[]).await?;
    
    println!("Sending PunchHoleRequest for {}...", target_id);
    let resp = client.punch_hole(target_id).await?;
    
    println!("Response:");
    println!("  failure: {:?}", resp.failure);
    println!("  is_udp: {}", resp.is_udp);
    println!("  relay_server: {}", resp.relay_server);
    
    if resp.failure != 0 {
        let failure_type = match resp.failure {
            f if f == Failure::IdNotExist as i32 => "ID_NOT_EXIST",
            f if f == Failure::Offline as i32 => "OFFLINE",
            _ => "UNKNOWN",
        };
        println!("Target peer status: FAILED ({})", failure_type);
    } else {
        println!("Target peer status: ONLINE");
    }
    
    Ok(())
}
