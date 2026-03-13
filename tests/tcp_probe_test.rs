#[path = "../src/proto.rs"]
mod proto;

use anyhow::Result;
use prost::Message;
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use std::time::Duration;
use proto::hbb::{RendezvousMessage, rendezvous_message, RegisterPeer};

#[tokio::test]
#[ignore]
async fn test_tcp_rendezvous_probe() -> Result<()> {
    let server_addr = "115.238.185.55:50076";
    println!("Connecting to TCP {}...", server_addr);
    
    let mut stream = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(server_addr)
    ).await??;
    
    // Construct RegisterPeer message
    let register_peer = RegisterPeer {
        id: "rustdesk-cli-probe-tcp".to_string(),
        serial: 0,
    };
    let rendezvous_msg = RendezvousMessage {
        union: Some(rendezvous_message::Union::RegisterPeer(register_peer)),
    };
    
    let mut buf = Vec::new();
    rendezvous_msg.encode(&mut buf)?;
    
    // TCP messages in RustDesk are prefixed with 4-byte LE length
    let len = buf.len() as u32;
    let mut packet = len.to_le_bytes().to_vec();
    packet.extend_from_slice(&buf);
    
    println!("Sending TCP request ({} bytes)...", packet.len());
    stream.write_all(&packet).await?;
    
    let mut len_buf = [0u8; 4];
    println!("Waiting for TCP response length...");
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut len_buf)).await??;
    let resp_len = u32::from_le_bytes(len_buf);
    println!("Response length: {}", resp_len);
    
    let mut resp_buf = vec![0u8; resp_len as usize];
    stream.read_exact(&mut resp_buf).await?;
    
    let decoded = RendezvousMessage::decode(&resp_buf[..])?;
    println!("Decoded TCP response: {:?}", decoded);
    
    Ok(())
}
