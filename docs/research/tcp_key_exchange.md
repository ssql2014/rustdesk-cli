# Research: TCP KeyExchange Protocol (hbbs)

This document details the TCP KeyExchange protocol used by the RustDesk rendezvous server (`hbbs`). This process is required when a server is configured with a security key (`id_ed25519.pub`).

## 1. The Trigger: Why hbbs sends KeyExchange

When a client connects to `hbbs` via TCP (default port 21116) and attempts to send a message (like `PunchHoleRequest`) in plaintext, the server may respond with a `KeyExchange` message.

- **Condition:** The server has a public key configured (`id_ed25519.pub` exists in the server directory).
- **Behavior:** The server mandates an encrypted channel for TCP signaling to protect sensitive data like authentication tokens. If the first message received is not a valid handshake or if the server is configured to initiate, it sends its public key first.

## 2. Protocol Handshake Sequence

The official RustDesk client (`src/rendezvous_mediator.rs` and `src/common.rs`) implements this via a function called `secure_tcp`.

### Step 1: Client Connection
The client establishes a raw TCP connection to `hbbs:21116`.

### Step 2: Server Hello (KeyExchange)
The server sends a `RendezvousMessage` containing the `KeyExchange` variant (index 25).
- `key_exchange.keys[0]`: Contains the server's **permanent Ed25519 public key**.

### Step 3: Client Key Generation
Upon receiving the server's public key, the client:
1. Generates a random **32-byte symmetric key**.
2. This key will be used for `ChaCha20-Poly1305` encryption of the stream.

### Step 4: Client Response (KeyExchange)
The client sends back a `RendezvousMessage` with the `KeyExchange` variant.
- `key_exchange.keys[0]`: Contains a **NaCl Sealed Box** (`crypto_box_seal`) payload.
- The payload is the 32-byte symmetric key encrypted using the server's public key.
- *Note:* A Sealed Box automatically includes the ephemeral public key required by the server to decrypt the message.

### Step 5: Upgrade to Encrypted Stream
Both parties now initialize an `EncryptedStream` using the shared 32-byte symmetric key. 
- All subsequent `RendezvousMessage` objects are sent as encrypted payloads.
- The framing (length header) remains the same as `TcpTransport`.

### Step 6: Signaling
The client now sends the `PunchHoleRequest` (containing the actual session token) through the encrypted channel.

## 3. Protobuf Definition Reference

```protobuf
message KeyExchange {
  repeated bytes keys = 1; 
}

message RendezvousMessage {
  oneof union {
    ...
    KeyExchange key_exchange = 25;
    ...
  }
}
```

## 4. Implementation Requirements for rustdesk-cli

To resolve issue #38, `rustdesk-cli` must:
1. **Detect KeyExchange:** Modify the TCP connection logic to check if the first message from `hbbs` is a `KeyExchange`.
2. **Perform NaCl Handshake:**
   - Use `dryoc` or `sodiumoxide` to handle the Ed25519/X25519 math.
   - Implement `crypto_box_seal` to encrypt the symmetric key.
3. **Switch Transport:** Wrap the `TcpTransport` in an `EncryptedStream` decorator once the handshake completes.
4. **Resend Request:** Re-issue the `PunchHoleRequest` over the new encrypted stream.

---
*Note: This research confirms that TCP rendezvous is not simply "plaintext Protobuf" when a server key is present. It requires a mandatory security upgrade before any functional requests are processed.*
