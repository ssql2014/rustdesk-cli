# Secure TCP Stream Encryption Details

This document details the exact encryption and framing mechanism used by RustDesk for secure TCP streams after the `KeyExchange` handshake is completed.

## 1. Encryption Cipher

RustDesk uses **XSalsa20-Poly1305** for symmetric encryption of the TCP stream.

- **Implementation:** Leverages `sodiumoxide::crypto::secretbox` (libsodium).
- **Algorithm:** XSalsa20 stream cipher combined with Poly1305 MAC for authenticated encryption.
- **Key Size:** 32 bytes (`secretbox::KEYBYTES`).
- **Nonce Size:** 24 bytes (`secretbox::NONCEBYTES`).

## 2. Nonce Strategy

The encryption uses a **counter-based nonce** strategy to ensure uniqueness for every message.

- **Sequence Numbers:** Each `FramedStream` maintains two independent 64-bit counters (initialized to 0):
    - `send_seqnum`: Incremented before each outgoing message is encrypted.
    - `recv_seqnum`: Incremented before each incoming message is decrypted.
- **Nonce Construction:** The 24-byte nonce is constructed as follows:
    1. The first 8 bytes contain the little-endian representation of the sequence number.
    2. The remaining 16 bytes are padded with zeros.
- **Code Reference (`hbb_common/src/tcp.rs`):**
  ```rust
  fn get_nonce(seqnum: u64) -> Nonce {
      let mut nonce = Nonce([0u8; secretbox::NONCEBYTES]);
      nonce.0[..8].copy_from_slice(&seqnum.to_le_bytes());
      nonce
  }
  ```

## 3. Framing and Protocol Ordering

Encryption is applied to the **payload** of the frames, while the framing itself remains in plaintext.

### Framing Logic (`BytesCodec`)
The stream uses a custom variable-length length prefix:
- **Header Size:** 1 to 4 bytes.
- **Indicator:** The low 2 bits of the first byte encode the header length ($N = (\text{byte} \& 0x3) + 1$).
- **Length Value:** The actual payload length is $(\text{header\_value} >> 2)$.
- **Encoding:** Little-endian.

### Message Flow
1. **Outgoing:**
    - Plaintext (Protobuf bytes) $\rightarrow$ Encrypted with `secretbox::seal` using current `send_seqnum`.
    - Ciphertext $\rightarrow$ Framed with `BytesCodec` (adds length prefix).
    - Framed Ciphertext $\rightarrow$ Sent over TCP.
2. **Incoming:**
    - `BytesCodec` reads length prefix $\rightarrow$ Extracts the ciphertext frame.
    - Ciphertext $\rightarrow$ Decrypted with `secretbox::open` using current `recv_seqnum`.
    - Resulting Plaintext $\rightarrow$ Decoded as Protobuf.

## 4. Implementation Requirements

To maintain compatibility with `hbb_common`:
- Increment counters **immediately before** encryption/decryption. The first message in either direction will use sequence number `1`.
- Ensure the `secretbox` key is exactly 32 bytes (derived from the `KeyExchange` sealed box).
- Use little-endian for the sequence number inside the nonce.
