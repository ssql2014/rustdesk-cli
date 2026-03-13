#![allow(dead_code)]

//! Live integration test: connect to the real RustDesk server and open a terminal.
//!
//! Uses `connection::connect_to_peer` then `terminal::open_terminal` against the
//! self-hosted server. Marked `#[ignore]` so it only runs when explicitly requested.

#[path = "../src/proto.rs"]
mod proto;
#[path = "../src/rendezvous.rs"]
mod rendezvous;
#[path = "../src/transport.rs"]
mod transport;
#[path = "../src/crypto.rs"]
mod crypto;
#[path = "../src/connection.rs"]
mod connection;
#[path = "../src/terminal.rs"]
mod terminal;

use anyhow::Result;
use tokio::time::{Duration, timeout};

use connection::{ConnectionConfig, connect_to_peer};
use terminal::{open_terminal, close_terminal, send_terminal_data, recv_terminal_data, TerminalEvent};

const ID_SERVER_ADDR: &str = "115.238.185.55:50076";
const RELAY_SERVER_ADDR: &str = "115.238.185.55:50077";
const SERVER_KEY: &str = "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=";
const TARGET_PEER_ID: &str = "308235080";
const TARGET_PASSWORD: &str = "Evas@2026";

fn test_config() -> ConnectionConfig {
    ConnectionConfig {
        id_server: ID_SERVER_ADDR.to_string(),
        relay_server: RELAY_SERVER_ADDR.to_string(),
        server_key: SERVER_KEY.to_string(),
        peer_id: TARGET_PEER_ID.to_string(),
        password: TARGET_PASSWORD.to_string(),
        warmup_secs: 5,
    }
}

/// Connect to the live server, open a terminal, and print the TerminalOpened response.
#[tokio::test]
#[ignore = "hits live server"]
async fn live_open_terminal() -> Result<()> {
    let config = test_config();

    // Phase 1: Full connection (rendezvous → relay → crypto → auth).
    let result = timeout(Duration::from_secs(30), connect_to_peer(&config))
        .await
        .map_err(|_| anyhow::anyhow!("connect_to_peer timed out after 30s"))??;

    println!("Connected to peer:");
    println!("  hostname: {}", result.peer_info.hostname);
    println!("  platform: {}", result.peer_info.platform);
    println!("  displays: {}", result.peer_info.displays.len());

    let mut encrypted = result.encrypted;

    // Phase 2: Open terminal (24 rows x 80 cols).
    let terminal_info = timeout(Duration::from_secs(15), open_terminal(&mut encrypted, 24, 80))
        .await
        .map_err(|_| anyhow::anyhow!("open_terminal timed out after 15s"))??;

    println!("Terminal opened:");
    println!("  terminal_id: {}", terminal_info.terminal_id);
    println!("  pid: {}", terminal_info.pid);
    println!("  service_id: {}", terminal_info.service_id);

    assert!(terminal_info.terminal_id >= 0, "terminal_id should be non-negative");
    assert!(terminal_info.pid > 0, "pid should be positive");

    // Phase 3: Close the terminal.
    timeout(
        Duration::from_secs(5),
        close_terminal(&mut encrypted, terminal_info.terminal_id),
    )
    .await
    .map_err(|_| anyhow::anyhow!("close_terminal timed out after 5s"))??;

    println!("Terminal closed cleanly.");
    Ok(())
}

/// Connect, open terminal, run `echo` command, and verify output.
#[tokio::test]
#[ignore = "hits live server"]
async fn live_terminal_exec_echo() -> Result<()> {
    let config = test_config();

    let result = timeout(Duration::from_secs(30), connect_to_peer(&config))
        .await
        .map_err(|_| anyhow::anyhow!("connect_to_peer timed out after 30s"))??;

    let mut encrypted = result.encrypted;

    let terminal_info = timeout(Duration::from_secs(15), open_terminal(&mut encrypted, 24, 80))
        .await
        .map_err(|_| anyhow::anyhow!("open_terminal timed out after 15s"))??;

    println!("Terminal opened (id={}, pid={})", terminal_info.terminal_id, terminal_info.pid);

    // Send a simple echo command.
    send_terminal_data(&mut encrypted, terminal_info.terminal_id, b"echo rustdesk-cli-dv-test\n")
        .await?;

    // Collect output with idle timeout.
    let mut output = Vec::new();
    let idle = Duration::from_secs(3);
    loop {
        let recv_fut = recv_terminal_data(&mut encrypted);
        match tokio::time::timeout(idle, recv_fut).await {
            Ok(Ok(TerminalEvent::Data(chunk))) => {
                let text = String::from_utf8_lossy(&chunk);
                println!("  recv: {:?}", text);
                output.extend_from_slice(&chunk);
            }
            Ok(Ok(TerminalEvent::Closed { exit_code })) => {
                println!("  terminal closed with exit_code={exit_code}");
                break;
            }
            Ok(Ok(TerminalEvent::Error(msg))) => {
                println!("  terminal error: {msg}");
                break;
            }
            Ok(Err(e)) => {
                println!("  recv error: {e:#}");
                break;
            }
            Err(_) => {
                println!("  idle timeout — done collecting");
                break;
            }
        }
    }

    let output_str = String::from_utf8_lossy(&output);
    println!("Full output: {:?}", output_str);
    assert!(
        output_str.contains("rustdesk-cli-dv-test"),
        "output should contain the echo string, got: {output_str:?}"
    );

    // Cleanup.
    let _ = close_terminal(&mut encrypted, terminal_info.terminal_id).await;
    Ok(())
}
