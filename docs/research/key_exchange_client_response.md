# Research: Exact Client KeyExchange Response Format

This document details the exact format the official RustDesk client uses for the client-side KeyExchange response, based on analysis of `src/common.rs::create_symmetric_key_msg` and `libs/hbb_common/src/tcp.rs::FramedStream::set_key`.

## 1. Handshake Encryption: `crypto_box` (NOT `crypto_box_seal`)

**Critical finding:** The client uses **authenticated encryption** (`crypto_box`), NOT anonymous sealed boxes (`crypto_box_seal`).

- **Their Public Key:** The server's **ephemeral X25519 public key** from the server's `KeyExchange.keys[0]`. This key must first be verified against the server's permanent Ed25519 public key (the `--key` parameter / `id_ed25519.pub`).
- **Our Secret Key:** The client generates an **ephemeral X25519 keypair** for the handshake.
- **Payload:** The 32-byte symmetric key that will be used for the subsequent stream.
- **Nonce:** All-zero nonce: `[0u8; 24]`.

### Why `crypto_box_seal` is wrong

`crypto_box_seal` embeds the ephemeral public key inside the sealed box itself and uses a derived nonce. The server expects `crypto_box` format — it already has the client's ephemeral public key from `keys[0]` and uses a zero nonce. Using `crypto_box_seal` produces a different ciphertext format that the server cannot decrypt.

## 2. Client `KeyExchange.keys[]` Format

The client's `KeyExchange.keys` array must contain **exactly two entries**:

| Index | Content | Size |
|-------|---------|------|
| `keys[0]` | Client's ephemeral X25519 **public key** | 32 bytes |
| `keys[1]` | `crypto_box(symmetric_key, zero_nonce, server_ephemeral_pk, client_ephemeral_sk)` | 48 bytes (32 + 16 tag) |

### `create_symmetric_key_msg` pseudocode

```rust
fn create_symmetric_key_msg(their_pk: [u8; 32]) -> (PublicKey, Vec<u8>, SecretboxKey) {
    let (our_pk, our_sk) = crypto_box::gen_keypair();
    let symmetric_key = secretbox::gen_key();
    let nonce = [0u8; 24];
    let sealed = crypto_box::seal(&symmetric_key.0, &nonce, &their_pk, &our_sk);
    // Returns: our public key, the sealed ciphertext, the symmetric key
    (our_pk, sealed, symmetric_key)
}
```

The caller then constructs:
```rust
KeyExchange {
    keys: vec![our_pk.as_bytes().to_vec(), sealed],
}
```

## 3. Post-Handshake Stream Encryption

- **Cipher:** XSalsa20-Poly1305 (`libsodium secretbox` / `sodiumoxide::crypto::secretbox`)
- **Key:** The 32-byte symmetric key from the handshake
- **Nonce Strategy:** Counter-based sequence numbers

### Nonce Construction

Two independent 64-bit counters: one for send, one for receive. Both initialize to **0**.

**The counter is incremented BEFORE each encrypt/decrypt.** The first encrypted message in either direction uses sequence number **1**.

```
nonce = seq_number_le64 || [0u8; 16]     // 8 + 16 = 24 bytes
```

Where `seq_number_le64` is the 8-byte little-endian representation of the counter.

### Sequence per direction

| Direction | Counter | First message seq |
|-----------|---------|-------------------|
| Client → Server (send) | `send_seq` | 1 |
| Server → Client (recv) | `recv_seq` | 1 |

## 4. Framing

The encryption is applied to the **payload only**. The BytesCodec length-prefix (1-4 bytes) remains in **plaintext** and prefixes the encrypted ciphertext.

```
[plaintext length header] [encrypted payload (ciphertext + poly1305 tag)]
```

## 5. What Our Code Does Wrong

Our `complete_tcp_key_exchange()` in `src/rendezvous.rs` has these bugs:

1. **Wrong encryption function:** Uses `crypto_box_seal` (anonymous sealed box) instead of `crypto_box` (authenticated box with zero nonce). The server cannot decrypt `crypto_box_seal` output because it expects `crypto_box` format.

2. **Wrong number of keys:** Sends only 1 entry in `keys[]` (the sealed box). Must send 2: `[client_ephemeral_pk, crypto_box_ciphertext]`.

3. **Counter initialization:** Must ensure the encrypted stream starts counters at 0 and increments before first use (first message = seq 1).

## 6. Corrected Handshake Sequence

1. Receive server's `KeyExchange { keys: [server_ephemeral_pk] }`
2. Verify `server_ephemeral_pk` signature against permanent server Ed25519 key
3. Generate client ephemeral X25519 keypair: `(client_pk, client_sk)`
4. Generate random 32-byte symmetric key
5. Encrypt: `sealed = crypto_box(symmetric_key, [0u8;24], server_ephemeral_pk, client_sk)`
6. Send: `KeyExchange { keys: [client_pk, sealed] }`
7. Initialize encrypted stream with symmetric key, send_seq=0, recv_seq=0
8. Re-send `PunchHoleRequest` over encrypted stream

---
*Research by Nova (Gemini), 2026-03-15. Based on analysis of rustdesk/rustdesk `src/common.rs` and `libs/hbb_common/src/tcp.rs`.*
