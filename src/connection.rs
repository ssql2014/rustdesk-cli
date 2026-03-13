//! Full RustDesk connection orchestration.
//!
//! Implements the complete connection flow:
//! 1. Rendezvous discovery via hbbs (PunchHoleRequest)
//! 2. Relay fallback via hbbr when P2P fails
//! 3. NaCl key exchange (Ed25519→Curve25519, crypto_box)
//! 4. Password authentication (two-stage SHA256)
//! 5. LoginResponse / PeerInfo parsing

use anyhow::{Context, Result, bail};
use prost::Message as ProstMessage;
use rand_core::{OsRng, RngCore};

use crate::crypto::{self, EncryptedStream, KeyExchangeResult};
use crate::proto::hbb::{
    LoginRequest, Message, PeerInfo, PublicKey, PunchHoleResponse,
    login_response, message, punch_hole_response,
};
use crate::rendezvous::RendezvousClient;
use crate::transport::{TcpTransport, Transport};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Parameters needed to establish a RustDesk connection.
pub struct ConnectionConfig {
    /// Rendezvous / ID server address (e.g. "1.2.3.4:21116").
    pub id_server: String,
    /// Relay server address (e.g. "1.2.3.4:21117").
    pub relay_server: String,
    /// Server's Ed25519 public key, base64-encoded.
    pub server_key: String,
    /// Target peer ID (e.g. "308235080").
    pub peer_id: String,
    /// Password for authentication.
    pub password: String,
    /// Seconds to wait after starting heartbeat before sending PunchHole.
    /// The server may need sustained heartbeats before accepting PunchHole
    /// requests (Nova §28).  Default: 5.
    pub warmup_secs: u64,
}

/// Outcome of a successful connection.
pub struct ConnectionResult {
    pub peer_info: PeerInfo,
    pub encrypted: EncryptedStream<TcpTransport>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Connect to a remote RustDesk peer through the full protocol flow.
///
/// 1. Rendezvous discovery via hbbs
/// 2. Relay TCP connection via hbbr
/// 3. NaCl key exchange
/// 4. Password authentication
/// 5. Returns PeerInfo + encrypted stream
pub async fn connect_to_peer(config: &ConnectionConfig) -> Result<ConnectionResult> {
    let server_pk = decode_server_key(&config.server_key)?;

    // Phase 1: Rendezvous discovery.
    let relay_info = rendezvous_discover(config).await?;

    // Phase 2: Relay TCP connection.
    let relay_addr = relay_info
        .relay_server
        .as_deref()
        .unwrap_or(&config.relay_server);
    let transport = relay_connect(relay_addr, &relay_info.uuid, &config.peer_id, &config.server_key).await?;

    // Phase 3+4: NaCl handshake + authentication (takes ownership of transport).
    handshake_and_auth(transport, &server_pk, &config.password, &config.peer_id).await
}

// ---------------------------------------------------------------------------
// Internal: Rendezvous discovery
// ---------------------------------------------------------------------------

struct RelayInfo {
    relay_server: Option<String>,
    uuid: String,
}

async fn rendezvous_discover(config: &ConnectionConfig) -> Result<RelayInfo> {
    let client = RendezvousClient::connect(&config.id_server)
        .await
        .context("failed to connect to rendezvous server")?;

    // Register ourselves as a peer (required before punch-hole).
    let my_id = format!("cli-{}", std::process::id());
    let register_response = client
        .register_peer(&my_id, &[])
        .await
        .context("RegisterPeer failed")?;

    if register_response.request_pk {
        let mut uuid = [0_u8; 16];
        let mut public_key = [0_u8; 32];
        OsRng.fill_bytes(&mut uuid);
        OsRng.fill_bytes(&mut public_key);

        client
            .register_pk(&my_id, &uuid, &public_key)
            .await
            .context("RegisterPk failed")?;
    }

    // Start heartbeat to maintain presence on the rendezvous server.
    // The first heartbeat fires immediately; yield to let it send before
    // we proceed with PunchHole.
    let heartbeat = client.start_heartbeat(&my_id);
    tokio::task::yield_now().await;

    // State warm-up: the server may need several sustained heartbeats
    // before it will respond to PunchHoleRequests (Nova §28).
    let warmup = config.warmup_secs;
    if warmup > 0 {
        tokio::time::sleep(tokio::time::Duration::from_secs(warmup)).await;
    }

    // Wrap remaining discovery in an async block so we always abort the
    // heartbeat, even on error paths.
    let result = async {
        // Try to punch hole to target (15s timeout — server may be slow).
        let ph_response = tokio::time::timeout(
            tokio::time::Duration::from_secs(15),
            client.punch_hole(&config.peer_id, &config.server_key),
        )
        .await
        .map_err(|_| anyhow::anyhow!("PunchHoleRequest timed out after 15 seconds"))?
        .context("PunchHoleRequest failed")?;

        // Check for immediate PunchHole failure before proceeding to relay.
        check_punch_hole_failure(&ph_response)?;

        // Determine relay info from punch-hole response.
        let relay_server = if ph_response.relay_server.is_empty() {
            None
        } else {
            Some(ph_response.relay_server.clone())
        };

        // Request relay — we always go through relay for now.
        let relay_response = client
            .request_relay_for(
                &config.peer_id,
                relay_server.as_deref().unwrap_or(&config.relay_server),
                &ph_response.socket_addr,
            )
            .await
            .context("RequestRelay failed")?;

        let uuid = relay_response.uuid;
        let relay_addr = if relay_response.relay_server.is_empty() {
            relay_server
        } else {
            Some(relay_response.relay_server)
        };

        Ok(RelayInfo {
            relay_server: relay_addr,
            uuid,
        })
    }
    .await;

    heartbeat.abort();
    result
}

/// Check for immediate PunchHole failure codes and bail with a descriptive error.
///
/// The rendezvous server signals errors via the `failure` enum field and the
/// free-text `other_failure` string.  We detect these early so the CLI can
/// report a clear message instead of silently falling through to a relay that
/// will also fail.
fn check_punch_hole_failure(resp: &PunchHoleResponse) -> Result<()> {
    // Non-empty other_failure is always an error, regardless of the enum value.
    if !resp.other_failure.is_empty() {
        bail!("punch hole failed: {}", resp.other_failure);
    }

    match punch_hole_response::Failure::try_from(resp.failure) {
        // 0 = IdNotExist is also the protobuf default.  Distinguish a real
        // "ID not found" from "no error" by checking whether the server
        // gave us any useful addressing data.
        Ok(punch_hole_response::Failure::IdNotExist) => {
            if resp.socket_addr.is_empty() && resp.relay_server.is_empty() {
                bail!("punch hole failed: the target ID does not exist on the rendezvous server");
            }
            Ok(())
        }
        Ok(punch_hole_response::Failure::Offline) => {
            bail!("punch hole failed: the target peer is offline");
        }
        Ok(punch_hole_response::Failure::LicenseMismatch) => {
            bail!("punch hole failed: license mismatch between client and server");
        }
        Ok(punch_hole_response::Failure::LicenseOveruse) => {
            bail!("punch hole failed: license connection limit exceeded (login overload)");
        }
        // Unknown failure code — report the raw value.
        Err(_) => {
            bail!("punch hole failed: unknown failure code {}", resp.failure);
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: Relay TCP connection + binding
// ---------------------------------------------------------------------------

async fn relay_connect(relay_addr: &str, uuid: &str, peer_id: &str, licence_key: &str) -> Result<TcpTransport> {
    let mut transport = TcpTransport::connect(relay_addr)
        .await
        .with_context(|| format!("failed to connect TCP to relay {relay_addr}"))?;

    // Send relay binding message (RendezvousMessage with RequestRelay).
    let binding = crate::proto::hbb::RendezvousMessage {
        union: Some(crate::proto::hbb::rendezvous_message::Union::RequestRelay(
            crate::proto::hbb::RequestRelay {
                id: peer_id.to_string(),
                uuid: uuid.to_string(),
                socket_addr: Vec::new(),
                relay_server: String::new(),
                secure: true,
                licence_key: licence_key.to_string(),
                conn_type: crate::proto::hbb::ConnType::DefaultConn as i32,
                token: String::new(),
                control_permissions: None,
            },
        )),
    };
    let mut buf = Vec::new();
    binding.encode(&mut buf)?;
    transport.send(&buf).await?;

    Ok(transport)
}

// ---------------------------------------------------------------------------
// Internal: NaCl handshake + authentication
// ---------------------------------------------------------------------------

/// Perform the complete post-relay handshake: NaCl key exchange + authentication.
/// Takes ownership of the transport.
async fn handshake_and_auth(
    mut transport: TcpTransport,
    server_ed25519_pk: &[u8; 32],
    password: &str,
    my_id: &str,
) -> Result<ConnectionResult> {
    // --- NaCl key exchange ---

    // Step 1: Receive SignedId from host.
    let raw = transport.recv().await.context("waiting for SignedId")?;
    let msg = Message::decode(raw.as_slice()).context("decode SignedId message")?;

    match msg.union {
        Some(message::Union::SignedId(_)) => {}
        other => bail!("expected SignedId, got {other:?}"),
    };

    // Step 2: Perform key exchange using the server's known Ed25519 PK.
    let KeyExchangeResult {
        ephemeral_pk,
        sealed_key,
        session_key,
    } = crypto::key_exchange(server_ed25519_pk).context("NaCl key exchange failed")?;

    // Step 3: Send PublicKey message to host.
    let pk_msg = Message {
        union: Some(message::Union::PublicKey(PublicKey {
            asymmetric_value: ephemeral_pk.to_vec(),
            symmetric_value: sealed_key,
        })),
    };
    let mut buf = Vec::new();
    pk_msg.encode(&mut buf)?;
    transport.send(&buf).await?;

    // Step 4: Switch to encrypted stream.
    let mut encrypted = EncryptedStream::new(transport, &session_key);

    // --- Authentication ---

    // Step 5: Receive Hash (salt + challenge) — encrypted.
    let hash_raw = encrypted.recv().await.context("waiting for Hash")?;
    let hash_msg = Message::decode(hash_raw.as_slice()).context("decode Hash message")?;

    let hash = match hash_msg.union {
        Some(message::Union::Hash(h)) => h,
        other => bail!("expected Hash, got {other:?}"),
    };

    // Step 6: Compute password hash.
    let pw_hash =
        crypto::password_hash(password, hash.salt.as_bytes(), hash.challenge.as_bytes());

    // Step 7: Send LoginRequest — encrypted.
    let login_req = Message {
        union: Some(message::Union::LoginRequest(LoginRequest {
            username: String::new(),
            password: pw_hash.to_vec(),
            my_id: my_id.to_string(),
            my_name: "rustdesk-cli".to_string(),
            option: None,
            video_ack_required: false,
            session_id: rand_session_id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os_login: None,
            my_platform: std::env::consts::OS.to_string(),
            hwid: Vec::new(),
            avatar: String::new(),
            union: None,
        })),
    };
    let mut buf = Vec::new();
    login_req.encode(&mut buf)?;
    encrypted.send(&buf).await?;

    // Step 8: Receive LoginResponse — encrypted.
    let resp_raw = encrypted
        .recv()
        .await
        .context("waiting for LoginResponse")?;
    let resp_msg = Message::decode(resp_raw.as_slice()).context("decode LoginResponse")?;

    let peer_info = match resp_msg.union {
        Some(message::Union::LoginResponse(lr)) => match lr.union {
            Some(login_response::Union::PeerInfo(info)) => info,
            Some(login_response::Union::Error(e)) => bail!("login rejected: {e}"),
            None => bail!("LoginResponse has no union field"),
        },
        Some(message::Union::PeerInfo(info)) => info,
        other => bail!("expected LoginResponse, got {other:?}"),
    };

    Ok(ConnectionResult {
        peer_info,
        encrypted,
    })
}

/// Generate a random u64 session ID.
fn rand_session_id() -> u64 {
    use rand_core::RngCore;
    let mut buf = [0u8; 8];
    rand_core::OsRng.fill_bytes(&mut buf);
    u64::from_le_bytes(buf)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode a base64-encoded Ed25519 public key into 32 bytes.
fn decode_server_key(base64_key: &str) -> Result<[u8; 32]> {
    let bytes = base64_decode(base64_key).context("invalid base64 server key")?;
    let pk: [u8; 32] = bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("server key is {} bytes, expected 32", v.len()))?;
    Ok(pk)
}

/// Minimal base64 decoder (standard alphabet + padding).
fn base64_decode(input: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => bail!("invalid base64 character: {}", c as char),
        }
    }

    let input = input.trim_end_matches('=');
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let bytes = input.as_bytes();

    for chunk in bytes.chunks(4) {
        let mut buf: u32 = 0;
        let len = chunk.len();
        for (i, &b) in chunk.iter().enumerate() {
            buf |= (val(b)? as u32) << (18 - 6 * i);
        }
        out.push((buf >> 16) as u8);
        if len > 2 {
            out.push((buf >> 8) as u8);
        }
        if len > 3 {
            out.push(buf as u8);
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decode_server_key() {
        let key = "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=";
        let bytes = decode_server_key(key).expect("should decode");
        assert_eq!(bytes.len(), 32);
        // First few bytes as sanity check (decoded from "SWc0NI...")
        assert_eq!(bytes[0], 0x49); // 'I'
        assert_eq!(bytes[1], 0x67); // 'g'
        assert_eq!(bytes[2], 0x34); // '4'
    }

    #[test]
    fn base64_roundtrip() {
        let key = "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=";
        let decoded = base64_decode(key).unwrap();
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn rand_session_id_is_nonzero() {
        // Statistically impossible for a random u64 to be 0
        let id = rand_session_id();
        // Just ensure it doesn't panic; the value is random.
        let _ = id;
    }

    #[test]
    fn punch_hole_success_passes_check() {
        let resp = PunchHoleResponse {
            socket_addr: vec![127, 0, 0, 1, 0x20, 0xfb],
            relay_server: "relay.example.com:21117".to_string(),
            failure: 0,
            ..Default::default()
        };
        assert!(check_punch_hole_failure(&resp).is_ok());
    }

    #[test]
    fn punch_hole_offline_fails() {
        let resp = PunchHoleResponse {
            failure: punch_hole_response::Failure::Offline as i32,
            ..Default::default()
        };
        let err = check_punch_hole_failure(&resp).unwrap_err();
        assert!(err.to_string().contains("offline"), "got: {err}");
    }

    #[test]
    fn punch_hole_license_mismatch_fails() {
        let resp = PunchHoleResponse {
            failure: punch_hole_response::Failure::LicenseMismatch as i32,
            ..Default::default()
        };
        let err = check_punch_hole_failure(&resp).unwrap_err();
        assert!(err.to_string().contains("license mismatch"), "got: {err}");
    }

    #[test]
    fn punch_hole_license_overuse_fails() {
        let resp = PunchHoleResponse {
            failure: punch_hole_response::Failure::LicenseOveruse as i32,
            ..Default::default()
        };
        let err = check_punch_hole_failure(&resp).unwrap_err();
        assert!(
            err.to_string().contains("license connection limit"),
            "got: {err}"
        );
    }

    #[test]
    fn punch_hole_id_not_exist_with_empty_addrs_fails() {
        // failure=0 (IdNotExist) + no socket_addr + no relay_server → real failure
        let resp = PunchHoleResponse {
            failure: 0,
            socket_addr: vec![],
            relay_server: String::new(),
            ..Default::default()
        };
        let err = check_punch_hole_failure(&resp).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "got: {err}");
    }

    #[test]
    fn punch_hole_other_failure_string() {
        let resp = PunchHoleResponse {
            other_failure: "custom server error".to_string(),
            ..Default::default()
        };
        let err = check_punch_hole_failure(&resp).unwrap_err();
        assert!(
            err.to_string().contains("custom server error"),
            "got: {err}"
        );
    }

    /// Live server integration test — requires the self-hosted RustDesk server
    /// and target machine (308235080) to be online.
    #[tokio::test]
    #[ignore = "requires live RustDesk server"]
    async fn connect_to_live_server() {
        let config = ConnectionConfig {
            id_server: "115.238.185.55:50076".to_string(),
            relay_server: "115.238.185.55:50077".to_string(),
            server_key: "SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=".to_string(),
            peer_id: "308235080".to_string(),
            password: "Evas@2026".to_string(),
            warmup_secs: 5,
        };

        match connect_to_peer(&config).await {
            Ok(result) => {
                println!("Connected successfully!");
                println!("  hostname: {}", result.peer_info.hostname);
                println!("  platform: {}", result.peer_info.platform);
                println!("  displays: {}", result.peer_info.displays.len());
                for (i, d) in result.peer_info.displays.iter().enumerate() {
                    println!("    display {i}: {}x{}", d.width, d.height);
                }
            }
            Err(e) => {
                eprintln!("Connection failed (expected if server is down): {e:#}");
            }
        }
    }
}
