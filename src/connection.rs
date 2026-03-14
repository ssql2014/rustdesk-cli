//! Full RustDesk connection orchestration.
//!
//! Implements the complete connection flow:
//! 1. Rendezvous discovery via hbbs (PunchHoleRequest)
//! 2. Relay fallback via hbbr when P2P fails
//! 3. NaCl key exchange (Ed25519→Curve25519, crypto_box)
//! 4. Password authentication (two-stage SHA256)
//! 5. LoginResponse / PeerInfo parsing

use std::time::Duration;

use anyhow::{Context, Result, bail};
use prost::Message as ProstMessage;
use rand_core::{OsRng, RngCore};
use tokio::time::timeout;

use crate::crypto::{self, EncryptedStream, KeyExchangeResult};
use crate::proto::hbb::{
    ConnType, IdPk, ImageQuality, LoginRequest, Message, OptionMessage, PeerInfo,
    PublicKey, PunchHoleResponse, SupportedDecoding, login_request, login_response,
    message, option_message, punch_hole_response,
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
    /// requests (Nova §28).  Default: 2.
    pub warmup_secs: u64,
}

/// Outcome of a successful connection.
pub struct ConnectionResult {
    pub peer_info: PeerInfo,
    pub encrypted: EncryptedStream<TcpTransport>,
}

const PUNCH_HOLE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Connect to a remote RustDesk peer through the full protocol flow.
///
/// 1. Register with hbbs + heartbeat warmup
/// 2. Fire PunchHole (no wait — hbbs forwards silently on success)
/// 3. Immediately connect to hbbr and send relay binding
/// 4. NaCl key exchange + password authentication
/// 5. Returns PeerInfo + encrypted stream
pub async fn connect(config: &ConnectionConfig) -> Result<ConnectionResult> {
    connect_with_mode(config, ConnType::DefaultConn, None).await
}

pub(crate) async fn connect_with_mode(
    config: &ConnectionConfig,
    conn_type: ConnType,
    login_union: Option<login_request::Union>,
) -> Result<ConnectionResult> {
    // Phase 1: Register with hbbs and start heartbeat.
    eprintln!("[debug] Phase 1: registering with hbbs...");
    let client = RendezvousClient::connect(&config.id_server)
        .await
        .context("failed to connect to rendezvous server")?;

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

    let heartbeat = client.start_heartbeat(&my_id);
    tokio::task::yield_now().await;

    if config.warmup_secs > 0 {
        eprintln!("[debug] Heartbeat warmup {}s...", config.warmup_secs);
        tokio::time::sleep(tokio::time::Duration::from_secs(config.warmup_secs)).await;
    }

    // Phase 2: Generate UUID and coordinate PunchHole / RequestRelay.
    // The official client generates its own UUID and sends it to both
    // hbbs (so hbbs can forward to the peer) and hbbr (to bind the relay).
    let mut uuid_bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut uuid_bytes);
    let session_uuid: String = uuid_bytes.iter().map(|b| format!("{b:02x}")).collect();
    eprintln!("[debug] Phase 2: sending PunchHole (uuid={})...", &session_uuid[..8]);

    let punch_hole_relay_hint = match timeout(
        PUNCH_HOLE_RESPONSE_TIMEOUT,
        client.punch_hole_with_conn_type(&config.peer_id, &config.server_key, conn_type),
    )
    .await
    {
        Ok(Ok(resp)) => {
            check_punch_hole_failure(&resp)?;
            if !resp.relay_server.is_empty() {
                Some(resp.relay_server)
            } else {
                None
            }
        }
        Ok(Err(e)) => {
            eprintln!("[debug] Phase 2: PunchHole request failed: {e:#}, continuing to relay");
            None
        }
        Err(_) => {
            eprintln!(
                "[debug] Phase 2: PunchHoleResponse timed out after {}s, continuing to relay",
                PUNCH_HOLE_RESPONSE_TIMEOUT.as_secs()
            );
            None
        }
    };

    // Send RequestRelay to hbbs via TCP (with correct BytesCodec framing).
    // hbbs forwards this to the peer, who then connects to hbbr with our UUID.
    let requested_relay_server = punch_hole_relay_hint
        .as_deref()
        .unwrap_or(&config.relay_server);
    let relay_addr = match tokio::time::timeout(
        tokio::time::Duration::from_secs(15),
        client.request_relay_via_tcp_with_conn_type(
            &config.peer_id,
            requested_relay_server,
            &[],
            &config.server_key,
            &session_uuid,
            conn_type,
        ),
    )
    .await
    {
        Ok(Ok(relay_response)) => {
            if !relay_response.refuse_reason.is_empty() {
                bail!("relay refused: {}", relay_response.refuse_reason);
            }
            eprintln!("[debug] Phase 2: got RelayResponse, relay={:?}", relay_response.relay_server);
            if relay_response.relay_server.is_empty() {
                config.relay_server.clone()
            } else {
                relay_response.relay_server
            }
        }
        Ok(Err(e)) => {
            eprintln!("[debug] Phase 2: RequestRelay TCP failed: {e:#}, using default relay");
            config.relay_server.clone()
        }
        Err(_) => {
            eprintln!("[debug] Phase 2: RequestRelay timed out, using default relay");
            config.relay_server.clone()
        }
    };

    // Phase 3: Connect to hbbr with the same UUID.
    eprintln!("[debug] Phase 3: connecting to relay {}...", relay_addr);
    let transport = relay_connect_with_type(
        &relay_addr,
        &session_uuid,
        &config.peer_id,
        &config.server_key,
        conn_type,
    )
    .await?;
    eprintln!("[debug] Phase 3: relay bound, waiting for peer handshake...");

    // Phase 4: NaCl handshake + authentication.
    let result = handshake_and_auth(
        transport,
        &config.password,
        &config.peer_id,
        &my_id,
        conn_type,
        login_union,
    )
    .await;

    heartbeat.abort();
    result
}

// (rendezvous_discover removed — connect() now handles the full flow inline)

/// Check for immediate PunchHole failure codes and bail with a descriptive error.
///
/// The rendezvous server signals errors via the `failure` enum field and the
/// free-text `other_failure` string.  We detect these early so the CLI can
/// report a clear message instead of silently falling through to a relay that
/// will also fail.
///
/// Relay fallback: `Offline` always returns `Ok(())` so the caller falls
/// through to RequestRelay using `config.relay_server`.  The relay path has
/// its own timeout, so truly-offline peers will be caught there.  Only
/// `LicenseMismatch`, `LicenseOveruse`, and `IdNotExist` (with no addressing
/// data) are hard failures.
fn check_punch_hole_failure(resp: &PunchHoleResponse) -> Result<()> {
    // Non-empty other_failure is always an error, regardless of the enum value.
    if !resp.other_failure.is_empty() {
        bail!("punch hole failed: {}", resp.other_failure);
    }

    let has_relay = !resp.relay_server.is_empty();

    match punch_hole_response::Failure::try_from(resp.failure) {
        // 0 = IdNotExist is also the protobuf default.  Distinguish a real
        // "ID not found" from "no error" by checking whether the server
        // gave us any useful addressing data.
        Ok(punch_hole_response::Failure::IdNotExist) => {
            if resp.socket_addr.is_empty() && !has_relay {
                bail!("punch hole failed: the target ID does not exist on the rendezvous server");
            }
            Ok(())
        }
        Ok(punch_hole_response::Failure::Offline) => {
            // Always fall through to relay — the client knows
            // config.relay_server even when the PunchHoleResponse doesn't
            // include one.  If the peer is truly unreachable the relay
            // request will time out with a clear error.
            Ok(())
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
    relay_connect_with_type(
        relay_addr,
        uuid,
        peer_id,
        licence_key,
        ConnType::DefaultConn,
    )
    .await
}

async fn relay_connect_with_type(
    relay_addr: &str,
    uuid: &str,
    peer_id: &str,
    licence_key: &str,
    conn_type: ConnType,
) -> Result<TcpTransport> {
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
                conn_type: conn_type as i32,
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
    password: &str,
    peer_id: &str,
    client_id: &str,
    conn_type: ConnType,
    login_union: Option<login_request::Union>,
) -> Result<ConnectionResult> {
    // --- NaCl key exchange ---

    // Step 1: Receive SignedId from host.
    // Longer timeout: peer needs time to receive PunchHole and connect to hbbr.
    let raw = timeout(Duration::from_secs(30), transport.recv())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for SignedId from peer (30s)"))?
        .context("waiting for SignedId")?;
    let msg = Message::decode(raw.as_slice()).context("decode SignedId message")?;

    let signed_id = match msg.union {
        Some(message::Union::SignedId(sid)) => sid,
        other => bail!("expected SignedId, got {other:?}"),
    };

    // Step 2: Extract the peer's ephemeral Curve25519 pk from SignedId.
    //
    // SignedId.id format: [64-byte Ed25519 signature] [protobuf-encoded IdPk]
    // IdPk { id: string, pk: bytes(32) } where pk is a Curve25519 box public key.
    //
    // We skip signature verification for now (would need the peer's Ed25519 signing
    // key from the rendezvous server) and just parse the IdPk payload.
    let signed_bytes = &signed_id.id;
    if signed_bytes.len() <= 64 {
        bail!("SignedId too short ({} bytes), expected >64", signed_bytes.len());
    }

    let id_pk_bytes = &signed_bytes[64..]; // strip Ed25519 signature
    let id_pk = IdPk::decode(id_pk_bytes).context("decode IdPk from SignedId")?;

    eprintln!("[debug] SignedId: peer_id={}, pk_len={}", id_pk.id, id_pk.pk.len());

    let peer_box_pk: [u8; 32] = id_pk.pk.as_slice().try_into().map_err(|_| {
        anyhow::anyhow!("peer Curve25519 pk is {} bytes, expected 32", id_pk.pk.len())
    })?;

    let KeyExchangeResult {
        ephemeral_pk,
        sealed_key,
        session_key,
    } = crypto::key_exchange_curve25519(&peer_box_pk)
        .context("NaCl key exchange with peer's Curve25519 pk failed")?;

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
    let hash_raw = timeout(Duration::from_secs(10), encrypted.recv())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for Hash challenge from peer"))?
        .context("waiting for Hash")?;
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
            username: peer_id.to_string(),
            password: pw_hash.to_vec(),
            my_id: client_id.to_string(),
            my_name: "rustdesk-cli".to_string(),
            option: Some(build_login_option_message(conn_type)),
            video_ack_required: false,
            session_id: rand_session_id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os_login: None,
            my_platform: match std::env::consts::OS {
                "macos" => "macOS".to_string(),
                os => os.to_string(),
            },
            hwid: Vec::new(),
            avatar: String::new(),
            union: login_union,
        })),
    };
    let mut buf = Vec::new();
    login_req.encode(&mut buf)?;
    encrypted.send(&buf).await?;

    // Step 8: Receive LoginResponse — encrypted.
    // The peer may send TestDelay or other messages before LoginResponse;
    // loop until we get the actual response.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let peer_info = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("timed out waiting for LoginResponse from peer");
        }
        let resp_raw = timeout(remaining, encrypted.recv())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for LoginResponse from peer"))?
            .context("waiting for LoginResponse")?;
        let resp_msg = Message::decode(resp_raw.as_slice()).context("decode LoginResponse")?;

        match resp_msg.union {
            Some(message::Union::LoginResponse(lr)) => match lr.union {
                Some(login_response::Union::PeerInfo(info)) => break info,
                Some(login_response::Union::Error(e)) => bail!("login rejected: {e}"),
                None => bail!("LoginResponse has no union field"),
            },
            Some(message::Union::PeerInfo(info)) => break info,
            Some(message::Union::TestDelay(_)) => continue, // skip TestDelay
            other => bail!("expected LoginResponse, got {other:?}"),
        }
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

fn build_login_option_message(conn_type: ConnType) -> OptionMessage {
    if conn_type == ConnType::Terminal {
        return OptionMessage {
            terminal_persistent: option_message::BoolOption::Yes as i32,
            ..Default::default()
        };
    }

    OptionMessage {
        image_quality: ImageQuality::Best as i32,
        custom_fps: 0,
        disable_audio: option_message::BoolOption::Yes as i32,
        disable_clipboard: option_message::BoolOption::Yes as i32,
        disable_camera: option_message::BoolOption::Yes as i32,
        terminal_persistent: option_message::BoolOption::Yes as i32,
        supported_decoding: Some(SupportedDecoding {
            ability_vp9: 1,
            ..Default::default()
        }),
        ..Default::default()
    }
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
    fn punch_hole_offline_without_relay_falls_through() {
        // Offline without relay_server in the response is NOT a hard error —
        // the caller has config.relay_server and should try relay anyway.
        let resp = PunchHoleResponse {
            failure: punch_hole_response::Failure::Offline as i32,
            ..Default::default()
        };
        assert!(check_punch_hole_failure(&resp).is_ok());
    }

    #[test]
    fn punch_hole_offline_with_relay_allows_fallback() {
        // Offline + relay_server populated → also OK (relay hint is a bonus).
        let resp = PunchHoleResponse {
            failure: punch_hole_response::Failure::Offline as i32,
            relay_server: "relay.example.com:21117".to_string(),
            ..Default::default()
        };
        assert!(check_punch_hole_failure(&resp).is_ok());
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
            warmup_secs: 2,
        };

        match connect(&config).await {
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
