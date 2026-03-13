#[path = "../src/proto.rs"]
mod proto;

use anyhow::Result;
use prost::Message;
use std::net::UdpSocket;
use std::time::Duration;
use proto::hbb::{RendezvousMessage, rendezvous_message, TestNatRequest};

#[tokio::test]
#[ignore]
async fn test_udp_nat_probe() -> Result<()> {
    let server_addr = "115.238.185.55:50076";
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    
    let msg = RendezvousMessage {
        union: Some(rendezvous_message::Union::TestNatRequest(TestNatRequest {
            serial: 1,
        })),
    };
    
    let mut buf = Vec::new();
    msg.encode(&mut buf)?;
    
    println!("Sending TestNatRequest to {}...", server_addr);
    socket.send_to(&buf, server_addr)?;
    
    let mut recv_buf = [0u8; 4096];
    match socket.recv_from(&mut recv_buf) {
        Ok((size, addr)) => {
            println!("Received response from {}: {:x?}", addr, &recv_buf[..size]);
            println!("Decoded: {:?}", RendezvousMessage::decode(&recv_buf[..size]));
        }
        Err(e) => println!("TestNatRequest failed: {}", e),
    }
    
    Ok(())
}
