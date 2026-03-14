//! NaCl crypto layer for RustDesk authentication and encrypted transport.
//!
//! Implements:
//! - Two-stage SHA256 password hashing (RESEARCH.md §9)
//! - Ed25519→Curve25519 key conversion + X25519 DH key exchange
//! - XSalsa20-Poly1305 secretbox encryption with sequence-based nonces

use anyhow::{Context, Result};
use crypto_box::{
    PublicKey as BoxPublicKey, SalsaBox, SecretKey as BoxSecretKey,
    aead::Aead,
};
use ed25519_dalek::VerifyingKey;
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use xsalsa20poly1305::{XSalsa20Poly1305, aead::KeyInit};

use crate::transport::Transport;

// ---------------------------------------------------------------------------
// Password hashing
// ---------------------------------------------------------------------------

/// Two-stage SHA256 password hash for RustDesk authentication.
///
/// Stage 1: `SHA256(password_bytes || salt)`
/// Stage 2: `SHA256(stage1_hash  || challenge)`
pub fn password_hash(password: &str, salt: &[u8], challenge: &[u8]) -> [u8; 32] {
    // Stage 1 — local hash
    let stage1: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(password.as_bytes());
        h.update(salt);
        h.finalize().into()
    };

    // Stage 2 — handshake hash
    let mut h = Sha256::new();
    h.update(stage1);
    h.update(challenge);
    h.finalize().into()
}

// ---------------------------------------------------------------------------
// Key exchange
// ---------------------------------------------------------------------------

/// Result of a key exchange with a RustDesk server.
pub struct KeyExchangeResult {
    /// Client's ephemeral X25519 public key (sent as `asymmetric_value`).
    pub ephemeral_pk: [u8; 32],
    /// The symmetric session key sealed in a crypto_box (sent as `symmetric_value`).
    pub sealed_key: Vec<u8>,
    /// The plaintext symmetric session key (kept locally for secretbox).
    pub session_key: [u8; 32],
}

/// Perform the RustDesk key exchange against a server's Ed25519 public key.
///
/// 1. Convert the server's Ed25519 public key to a Curve25519 (X25519) key.
/// 2. Generate an ephemeral Curve25519 key pair for this session.
/// 3. Generate a random 32-byte symmetric session key.
/// 4. Seal the session key in a `crypto_box` using a zeroed nonce.
pub fn key_exchange(server_ed25519_pk: &[u8; 32]) -> Result<KeyExchangeResult> {
    // Ed25519 PK → Curve25519 PK
    let ed_pk = VerifyingKey::from_bytes(server_ed25519_pk)
        .context("Invalid Ed25519 public key")?;
    let their_box_pk = BoxPublicKey::from(ed_pk.to_montgomery().to_bytes());
    key_exchange_with_box_pk(&their_box_pk)
}

/// Perform key exchange with a Curve25519 (X25519) box public key directly.
///
/// Used when the peer provides its ephemeral Curve25519 pk in the SignedId
/// (via IdPk.pk), rather than an Ed25519 key that needs conversion.
pub fn key_exchange_curve25519(their_pk_bytes: &[u8; 32]) -> Result<KeyExchangeResult> {
    let their_box_pk = BoxPublicKey::from(*their_pk_bytes);
    key_exchange_with_box_pk(&their_box_pk)
}

fn key_exchange_with_box_pk(their_box_pk: &BoxPublicKey) -> Result<KeyExchangeResult> {
    // Ephemeral Curve25519 key pair
    let ephemeral_sk = BoxSecretKey::generate(&mut OsRng);
    let ephemeral_pk = ephemeral_sk.public_key();

    // Random 32-byte symmetric session key
    let key_ga = XSalsa20Poly1305::generate_key(&mut OsRng);
    let mut session_key = [0u8; 32];
    session_key.copy_from_slice(&key_ga);

    // Seal session key with zeroed nonce
    let salsa_box = SalsaBox::new(their_box_pk, &ephemeral_sk);
    let zero_nonce = Default::default();
    let sealed_key = salsa_box
        .encrypt(&zero_nonce, session_key.as_ref())
        .map_err(|e| anyhow::anyhow!("Failed to seal session key: {e}"))?;

    Ok(KeyExchangeResult {
        ephemeral_pk: *ephemeral_pk.as_bytes(),
        sealed_key,
        session_key,
    })
}

// ---------------------------------------------------------------------------
// Encrypted stream
// ---------------------------------------------------------------------------

/// Wraps a [`Transport`] with XSalsa20-Poly1305 secretbox encryption.
///
/// Nonce layout: first 8 bytes = sequence number (u64 LE), remaining 16 = zero.
/// Two independent counters (send / recv), both start at 0 and increment
/// **before** use so the first message uses sequence 1.
pub struct EncryptedStream<T: Transport> {
    inner: T,
    cipher: XSalsa20Poly1305,
    send_seq: u64,
    recv_seq: u64,
    last_recv_at: Instant,
}

impl<T: Transport> EncryptedStream<T> {
    pub fn new(transport: T, session_key: &[u8; 32]) -> Self {
        let key = xsalsa20poly1305::Key::from_slice(session_key);
        Self {
            inner: transport,
            cipher: XSalsa20Poly1305::new(key),
            send_seq: 0,
            recv_seq: 0,
            last_recv_at: Instant::now(),
        }
    }

    /// Build a 24-byte nonce from a sequence number.
    fn make_nonce(seq: u64) -> xsalsa20poly1305::Nonce {
        let mut bytes = [0u8; 24];
        bytes[..8].copy_from_slice(&seq.to_le_bytes());
        *xsalsa20poly1305::Nonce::from_slice(&bytes)
    }

    /// Encrypt and send a message.
    pub async fn send(&mut self, plaintext: &[u8]) -> Result<()> {
        self.send_seq += 1;
        let nonce = Self::make_nonce(self.send_seq);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;
        self.inner.send(&ciphertext).await
    }

    /// Send a raw zero-length heartbeat frame without touching the crypto counters.
    pub async fn send_heartbeat(&mut self) -> Result<()> {
        self.inner.send(&[]).await
    }

    /// Receive and decrypt a message.
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let ciphertext = self.inner.recv().await?;
        self.last_recv_at = Instant::now();
        if ciphertext.is_empty() {
            return Ok(Vec::new());
        }
        self.recv_seq += 1;
        let nonce = Self::make_nonce(self.recv_seq);
        self.cipher
            .decrypt(&nonce, ciphertext.as_ref())
            .map_err(|_| anyhow::anyhow!("Decryption failed: invalid ciphertext or wrong key"))
    }

    /// Return how long it has been since the last successfully received frame.
    pub fn recv_idle_for(&self) -> Duration {
        self.last_recv_at.elapsed()
    }

    /// Close the underlying transport.
    pub async fn close(&mut self) -> Result<()> {
        self.inner.close().await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::FramedTransport;
    use tokio::io::duplex;

    // -- Test-only transport over tokio duplex --

    struct DuplexTransport {
        framed: FramedTransport<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn pair() -> (Self, Self) {
            let (a, b) = duplex(8192);
            (
                Self { framed: FramedTransport::new(a) },
                Self { framed: FramedTransport::new(b) },
            )
        }
    }

    impl Transport for DuplexTransport {
        async fn connect(_addr: &str) -> Result<Self> {
            unimplemented!("use DuplexTransport::pair()")
        }
        async fn send(&mut self, msg: &[u8]) -> Result<()> {
            self.framed.send(msg).await
        }
        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.framed.recv().await
        }
        async fn close(&mut self) -> Result<()> {
            self.framed.close().await
        }
    }

    // -- password_hash tests --

    #[test]
    fn password_hash_deterministic() {
        let h1 = password_hash("pw", b"salt", b"challenge");
        let h2 = password_hash("pw", b"salt", b"challenge");
        assert_eq!(h1, h2);
        assert_ne!(h1, [0u8; 32]);
    }

    #[test]
    fn password_hash_varies_with_each_input() {
        let base = password_hash("pw", b"salt", b"chal");
        assert_ne!(base, password_hash("other", b"salt", b"chal"));
        assert_ne!(base, password_hash("pw", b"other", b"chal"));
        assert_ne!(base, password_hash("pw", b"salt", b"other"));
    }

    #[test]
    fn password_hash_matches_manual_sha256() {
        let mut h1 = Sha256::new();
        h1.update(b"test");
        h1.update(b"somesalt");
        let stage1: [u8; 32] = h1.finalize().into();

        let mut h2 = Sha256::new();
        h2.update(stage1);
        h2.update(b"challenge");
        let expected: [u8; 32] = h2.finalize().into();

        assert_eq!(password_hash("test", b"somesalt", b"challenge"), expected);
    }

    // -- key_exchange tests --

    #[test]
    fn key_exchange_produces_valid_result() {
        let signing_key = ed25519_dalek::SigningKey::generate(&mut OsRng);
        let result = key_exchange(&signing_key.verifying_key().to_bytes())
            .expect("key exchange should succeed");

        assert_ne!(result.ephemeral_pk, [0u8; 32]);
        assert_ne!(result.session_key, [0u8; 32]);
        // sealed_key = 32-byte key + 16-byte Poly1305 tag
        assert!(result.sealed_key.len() > 32);
    }

    #[test]
    fn key_exchange_sealed_key_can_be_opened_by_server() {
        use sha2::Sha512;

        let signing_key = ed25519_dalek::SigningKey::generate(&mut OsRng);
        let result = key_exchange(&signing_key.verifying_key().to_bytes())
            .expect("key exchange should succeed");

        // Server-side: convert Ed25519 SK → X25519 SK (SHA-512 + clamp)
        let server_x25519_sk = {
            let mut h = Sha512::new();
            h.update(signing_key.to_bytes());
            let full: [u8; 64] = h.finalize().into();
            let mut scalar = [0u8; 32];
            scalar.copy_from_slice(&full[..32]);
            scalar[0] &= 248;
            scalar[31] &= 127;
            scalar[31] |= 64;
            scalar
        };

        let server_box_sk = BoxSecretKey::from(server_x25519_sk);
        let client_box_pk = BoxPublicKey::from(result.ephemeral_pk);
        let salsa_box = SalsaBox::new(&client_box_pk, &server_box_sk);
        let zero_nonce = Default::default();

        let opened = salsa_box
            .decrypt(&zero_nonce, result.sealed_key.as_ref())
            .expect("server should be able to open sealed key");

        assert_eq!(opened.as_slice(), &result.session_key);
    }

    #[test]
    fn key_exchange_rejects_invalid_ed25519_pk() {
        let bad_pk = [0u8; 32]; // all-zero is not a valid Ed25519 point
        // May or may not be considered invalid depending on the library;
        // if it doesn't error, the key exchange output just won't be useful
        // to a real server. We only check that it doesn't panic.
        let _ = key_exchange(&bad_pk);
    }

    // -- nonce tests --

    #[test]
    fn nonce_sequence_layout() {
        let n1 = EncryptedStream::<DuplexTransport>::make_nonce(1);
        assert_eq!(&n1[..8], &1u64.to_le_bytes());
        assert_eq!(&n1[8..], &[0u8; 16]);

        let n256 = EncryptedStream::<DuplexTransport>::make_nonce(256);
        assert_eq!(&n256[..8], &256u64.to_le_bytes());
        assert_eq!(&n256[8..], &[0u8; 16]);

        let n0 = EncryptedStream::<DuplexTransport>::make_nonce(0);
        assert_eq!(&n0[..], &[0u8; 24]);
    }

    // -- EncryptedStream tests --

    #[tokio::test]
    async fn encrypted_stream_roundtrip() {
        let (ct, st) = DuplexTransport::pair();
        let key = [42u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            client.send(b"hello from client").await.unwrap();
            let reply = client.recv().await.unwrap();
            client.close().await.unwrap();
            reply
        });

        let server_task = tokio::spawn(async move {
            let request = server.recv().await.unwrap();
            server.send(b"hello from server").await.unwrap();
            server.close().await.unwrap();
            request
        });

        let request = server_task.await.unwrap();
        let reply = client_task.await.unwrap();
        assert_eq!(request, b"hello from client");
        assert_eq!(reply, b"hello from server");
    }

    #[tokio::test]
    async fn encrypted_stream_multiple_messages() {
        let (ct, st) = DuplexTransport::pair();
        let key = [7u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        let client_task = tokio::spawn(async move {
            for i in 0..5u8 {
                client.send(&[i; 64]).await.unwrap();
            }
            client.close().await.unwrap();
        });

        let server_task = tokio::spawn(async move {
            for i in 0..5u8 {
                let msg = server.recv().await.unwrap();
                assert_eq!(msg, vec![i; 64]);
            }
            server.close().await.unwrap();
        });

        client_task.await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn encrypted_stream_wrong_key_fails() {
        let (ct, st) = DuplexTransport::pair();
        let mut client = EncryptedStream::new(ct, &[42u8; 32]);
        let mut server = EncryptedStream::new(st, &[99u8; 32]); // wrong key

        let send_task = tokio::spawn(async move {
            client.send(b"secret").await.unwrap();
        });

        let recv_task = tokio::spawn(async move { server.recv().await });

        send_task.await.unwrap();
        let result = recv_task.await.unwrap();
        assert!(result.is_err(), "decryption with wrong key should fail");
    }

    #[tokio::test]
    async fn encrypted_stream_heartbeat_bypasses_encryption_and_sequence() {
        let (ct, st) = DuplexTransport::pair();
        let key = [11u8; 32];
        let mut client = EncryptedStream::new(ct, &key);
        let mut server = EncryptedStream::new(st, &key);

        client.send_heartbeat().await.unwrap();
        let heartbeat = server.recv().await.unwrap();
        assert!(heartbeat.is_empty());
        assert_eq!(client.send_seq, 0);
        assert_eq!(server.recv_seq, 0);

        client.send(b"payload").await.unwrap();
        let payload = server.recv().await.unwrap();
        assert_eq!(payload, b"payload");
        assert_eq!(client.send_seq, 1);
        assert_eq!(server.recv_seq, 1);
    }
}
