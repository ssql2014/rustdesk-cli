#[path = "../src/proto.rs"]
mod proto;

use anyhow::Result;
use prost::Message;
use std::net::UdpSocket;
use std::time::Duration;
use proto::hbb::{RendezvousMessage, rendezvous_message, PunchHoleRequest};

#[tokio::test]
#[ignore]
async fn test_udp_punch_empty_key() -> Result<()> {
    let server_addr = "115.238.185.55:50076";
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    
    // Construct PunchHoleRequest with an EMPTY key
    let punch_hole = PunchHoleRequest {
        id: "308235080".to_string(),
        nat_type: 0,
        licence_key: "".to_string(),
        conn_type: 0,
        token: "".to_string(),
        version: "1.3.7".to_string(),
        udp_port: socket.local_addr()?.port() as i32,
        force_relay: false,
        upnp_port: 0,
        socket_addr_v6: vec![],
    };
    let rendezvous_msg = RendezvousMessage {
        union: Some(rendezvous_message::Union::PunchHoleRequest(punch_hole)),
    };
    
    let mut buf = Vec::new();
    rendezvous_msg.encode(&mut buf)?;
    
    println!("Sending PunchHoleRequest with EMPTY key...");
    socket.send_to(&buf, server_addr)?;
    
    let mut recv_buf = [0u8; 4096];
    match socket.recv_from(&mut recv_buf) {
        Ok((size, addr)) => {
            println!("Received response from {}:", addr);
            let decoded = RendezvousMessage::decode(&recv_buf[..size]);
            println!("Decoded: {:?}", decoded);
        }
        Err(e) => println!("Failed to get response with EMPTY key: {}", e),
    }
    
    Ok(())
}
