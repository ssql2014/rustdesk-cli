# RustDesk Protocol & Architecture Research

This document outlines the technical details required to build a minimal CLI client (`rustdesk-cli`) capable of connecting to remote machines via the RustDesk protocol, capturing screenshots, and sending keyboard/mouse inputs.

## 1. RustDesk Protocol & Transport

RustDesk uses a custom protocol built on **Protocol Buffers (Protobuf)** over **TCP** and **UDP**. It relies on a "Server Compound" for peer discovery and connectivity.

### Core Components
*   **hbbs (ID/Rendezvous Server):**
    *   **Port 21115 (TCP):** NAT type testing.
    *   **Port 21116 (TCP/UDP):** ID registration, heartbeat, and UDP hole punching.
*   **hbbr (Relay Server):**
    *   **Port 21117 (TCP):** Relay signaling.
    *   **Port 21118 (TCP):** Data relaying (fallback when P2P fails).

### Connection Flow
1.  **Registration:** Client registers its ID and public IP/port with `hbbs`.
2.  **Discovery:** Client requests the target host's details from `hbbs`.
3.  **Hole Punching:** `hbbs` sends `PunchHole` commands to both peers. They attempt direct UDP communication.
4.  **Direct P2P:** If successful, data flows directly via UDP.
5.  **Relay Fallback:** If hole punching fails, both peers connect to `hbbr` to tunnel traffic.

---

## 2. Authentication Flow

The security model is based on **NaCl (Networking and Cryptography library)** primitives.

### Cryptographic Handshake
1.  **Identity:** The server has a long-term **Ed25519** key pair. The public key is shared with the client.
2.  **Key Conversion:** Both parties convert their Ed25519 keys to **Curve25519 (X25519)** keys for Diffie-Hellman key exchange.
3.  **Ephemeral Exchange:** The client generates an ephemeral Curve25519 key pair and performs a key exchange with the server's public key to derive a **shared secret**.
4.  **Encryption:** Subsequent traffic is encrypted using **XSalsa20** (stream cipher) and authenticated with **Poly1305** (forming a NaCl `secretbox`).

---

## 3. Video & Input Handling

Data is multiplexed within Protobuf messages defined in `message.proto`.

### Video (Screen Capture)
*   **Codecs:** VP8, VP9, AV1 (default software), H264, H265 (hardware accelerated).
*   **Protobuf Message:** `VideoFrame` contains the encoded bytes and a `key` flag (for keyframes).
*   **CLI Strategy:** For screenshots, the client must be able to decode at least one I-frame (keyframe) from the stream using a library like `ffmpeg-next` or a codec-specific Rust crate.

### Input Injection
*   **Keyboard:** `KeyEvent` message.
    *   `key_code`: Raw hardware scancode.
    *   `characters`: UTF-8 string (Translation mode).
    *   `modifiers`: Shift, Ctrl, Alt states.
*   **Mouse:** `MouseEvent` message.
    *   `x`, `y`: Absolute coordinates on the remote screen.
    *   `mask`: Button states (Left: 1, Right: 2, Middle: 4).
    *   `is_mousedown`: Press vs. Release.

---

## 4. Key Source Files (RustDesk Repository)

The most important files for protocol implementation:

*   **`libs/hbb_common/protos/rendezvous.proto`:** Definitions for server signaling (Register, PunchHole, Relay).
*   **`libs/hbb_common/protos/message.proto`:** Definitions for session data (VideoFrame, KeyEvent, MouseEvent).
*   **`src/rendezvous_mediator.rs`:** The core logic for managing server connections and NAT traversal.
*   **`libs/hbb_common/src/socket_client.rs`:** Low-level TCP/UDP connection handling.
*   **`libs/hbb_common/src/config.rs`:** Default ports, server addresses, and key management.

---

## 5. Existing Rust Crates

*   **`hbb_common`:** (Internal to RustDesk) Highly recommended to use or reference as it contains all Protobuf definitions and common logic.
*   **`sodiumoxide`:** Rust bindings for libsodium (Ed25519, Curve25519, XSalsa20).
*   **`prost`:** Fast Protobuf implementation for Rust.
*   **`tokio` / `mio`:** For asynchronous networking.
*   **`enigo` / `scrap`:** Used by RustDesk for input/output; useful for understanding the OS-level mapping.

---

## 6. NAT Traversal Mechanism

RustDesk implements a "P2P-first" approach:
1.  **UDP Hole Punching:** Preferred for low latency.
2.  **TCP Hole Punching:** Attempted if UDP is restricted.
3.  **Relay (hbbr):** Used as a last resort. The relay server cannot decrypt the data as it lacks the shared secret established during the handshake.

For a minimal CLI, implementing the **Relay** path first might be easier for guaranteed connectivity, though **P2P** is necessary for performance.

---

## 7. Vendored Proto Analysis

The Protobuf definitions have been vendored into the `proto/` directory from the `hbb_common` repository.

### Source Information
*   **Repository:** `https://github.com/rustdesk/hbb_common` (Submodule of `rustdesk/rustdesk`)
*   **Commit Hash:** `980bc11` (February 14, 2026)
*   **Files:** `rendezvous.proto`, `message.proto`
*   **Imports:** None (Self-contained)

### Minimal Message Set for MVP

To achieve the goal of connecting, authenticating, capturing a screenshot, and sending input, the following messages are required:

#### Connection & Discovery (`rendezvous.proto`)
*   **`RendezvousMessage`**: The top-level wrapper for signaling.
*   **`RegisterPeer` / `RegisterPeerResponse`**: Registering the client with the ID server.
*   **`PunchHoleRequest` / `PunchHoleResponse`**: Attempting NAT traversal.
*   **`RequestRelay` / `RelayResponse`**: Fallback to relay server.
*   **`RegisterPk`**: Registering the public key for the session.

#### Session & Data (`message.proto`)
*   **`Message`**: The top-level wrapper for session data.
*   **`LoginRequest` / `LoginResponse`**: Authenticating with the remote host.
*   **`Hash`**: Handling challenge-response authentication.
*   **`VideoFrame` / `EncodedVideoFrame`**: Receiving screen data (VP8/VP9/AV1/H264/H265).
*   **`PeerInfo` / `DisplayInfo`**: Discovering remote screen resolutions and capabilities.
*   **`KeyEvent` / `ControlKey`**: Injecting keyboard input.
*   **`MouseEvent`**: Injecting mouse movement and clicks.
*   **`ScreenshotRequest` / `ScreenshotResponse`**: Optional alternative for capturing single frames.

---

## 8. Connection Sequence (Detailed)

This section traces the exact sequence of messages and cryptographic handshakes required to establish a session between a client and a remote host.

### Phase 1: Rendezvous & Discovery (via `hbbs`)
**Ports:** 21116 (UDP/TCP), 21115 (TCP)
1.  **Client → `hbbs` (`RegisterPk`)**: Client sends its ID, UUID, and Public Key (Ed25519) to the rendezvous server.
2.  **Client → `hbbs` (`RegisterPeer`)**: Client announces it is online and available.
3.  **Client → `hbbs` (`PunchHoleRequest`)**: Client requests a connection to a specific Target ID.
4.  **`hbbs` → Target (`PunchHole`)**: Server forwards the request to the target host.
5.  **Target → `hbbs` (`PunchHoleSent`)**: Target host attempts to "punch" a UDP hole and notifies the server.
6.  **`hbbs` → Client (`PunchHoleResponse`)**: Server provides the target's public/local IP and its public key.

### Phase 2: Secure Channel Handshake (NaCl Crypto)
Once IP addresses are exchanged, the client and host establish an encrypted tunnel before any sensitive data (like passwords) is sent.
1.  **Ephemeral Key Generation**: Both client and host generate ephemeral **Curve25519** key pairs for the session.
2.  **Key Exchange (`PublicKey`)**:
    *   Client sends its ephemeral Public Key.
    *   Host sends its ephemeral Public Key.
3.  **Shared Secret Derivation**:
    *   Both sides use their own private key and the other's public key to derive a 32-byte shared secret via **Diffie-Hellman (X25519)**.
    *   Using `libsodium`'s `crypto_box::precompute`, a `PrecomputedKey` is generated.
4.  **Symmetric Encryption**: All subsequent packets are encrypted using **XSalsa20-Poly1305** (NaCl `secretbox`) with a unique nonce for every message.

### Phase 3: Authentication & Session Initialization
**Protocol:** Protobuf over Encrypted TCP/UDP
1.  **Host → Client (`Hash`)**: Host sends a `salt` and a `challenge` string.
2.  **Client → Host (`LoginRequest`)**:
    *   `username`: Remote machine username.
    *   `password`: Hashed password using the provided `salt` and `challenge`.
    *   `my_id`: Client's unique ID.
    *   `version`: Client version string.
3.  **Host → Client (`PeerInfo`)**: If login is successful, the host sends its capabilities.
    *   `displays`: List of available screens and their resolutions.
    *   `features`: Supported features (terminal, privacy mode, etc.).
    *   `encoding`: Supported video codecs (VP8, VP9, AV1, H264).

### Phase 4: Steady State (Media & Input)
1.  **Host → Client (`VideoFrame`)**: The host continuously streams encoded video. The client MUST decode at least one keyframe to get a valid screenshot.
2.  **Client → Host (`KeyEvent`)**:
    *   `down`: Boolean (True for press, False for release).
    *   `control_key`: Enum (e.g., `Return`, `Escape`, `Shift`).
    *   `chr`: Scancode or Unicode.
3.  **Client → Host (`MouseEvent`)**:
    *   `x`, `y`: Absolute coordinates on the remote screen.
    *   `mask`: Bitmask for buttons (1=Left, 2=Right, 4=Middle).

### Phase 5: Relay Fallback (via `hbbr`)
**Ports:** 21117 (TCP), 21118 (TCP)
If Phase 1 (`PunchHole`) fails (e.g., due to symmetric NAT):
1.  **Client → `hbbs` (`RequestRelay`)**: Client asks for a relay server.
2.  **`hbbs` → Client (`RelayResponse`)**: Server provides the `hbbr` address and a unique `uuid` token.
3.  **Client/Host → `hbbr` (Connection)**: Both parties connect to the relay server using the same `uuid`.
*   **`hbbr` (Binding)**: The relay server bridges the two TCP streams.
*   **E2EE Tunnel**: The parties perform the same **Phase 2 (NaCl Handshake)** over the relay. `hbbr` only sees encrypted traffic and cannot decrypt it.

---

## 9. Crypto Implementation Details

This section details the specific cryptographic algorithms and message structures used by the RustDesk protocol.

### Password Hashing Algorithm
Authentication uses a two-stage SHA256 hashing process to verify the password without sending it in plain text.

1.  **Stage 1 (Local Hash)**:
    `password_bytes = SHA256(password_str + hash_message.salt)`
2.  **Stage 2 (Handshake Hash)**:
    `final_hash = SHA256(password_bytes + hash_message.challenge)`

The `final_hash` (32 bytes) is what is sent in the `LoginRequest.password` field.

### Key Conversion (Ed25519 to Curve25519)
RustDesk uses **Ed25519** for long-term identity and signing, but **Curve25519 (X25519)** for key exchange.
*   **Sign PK to Box PK**: `sodiumoxide::crypto::sign::ed25519_pk_to_curve25519`
*   **Sign SK to Box SK**: `sodiumoxide::crypto::sign::ed25519_sk_to_curve25519`
*   The server's public key (provided as base64 in the "Key" field) is initially an Ed25519 public key. It MUST be converted to a Curve25519 key to decrypt the initial handshake box.

### Ephemeral Key Exchange Flow
1.  **Peer Hello**: The host sends a `SignedId` message containing its identity and public key.
2.  **Verification**: The client verifies the signature using the server's long-term public key.
3.  **Client Key Generation**: The client generates an ephemeral Curve25519 key pair (`box_::gen_keypair`).
4.  **Symmetric Key Generation**: The client generates a random 32-byte symmetric key (`secretbox::gen_key`).
5.  **Sealing the Key**:
    *   The client creates an encrypted "box" using `box_::seal`.
    *   **Nonce**: A **zeroed nonce** (`[0u8; 24]`) is used for this specific handshake step.
    *   **Recipient PK**: The host's public key from the `SignedId` message.
    *   **Sender SK**: The client's ephemeral private key.
6.  **`PublicKey` Message**: The client sends a message containing:
    *   `asymmetric_value`: The client's ephemeral public key.
    *   `symmetric_value`: The sealed (encrypted) symmetric key.
7.  **Session Key**: The decrypted symmetric key becomes the session key for all subsequent `secretbox` encryption.

### Nonce Strategy for Secretbox
Once the symmetric session key is established, every packet is encrypted using `secretbox::seal`.
*   **Nonce Generation**: A 24-byte nonce is used.
*   **Structure**: The first 8 bytes of the nonce contain the **sequence number** (u64, little-endian). The remaining 16 bytes are zeroed.
*   **Increment**: There are two separate sequence numbers (counters): one for outgoing (encryption) and one for incoming (decryption). Both start at 0 and are incremented **before** use (the first packet uses sequence `1`).

### Rust Crates Used
*   **`sodiumoxide`**: Primary crypto library (wrapping libsodium).
*   **`sha2`**: For SHA256 hashing.
*   **`prost`**: For Protobuf serialization.

### `LoginRequest` Protobuf Fields
The following fields are typically populated in a `LoginRequest`:
*   `username` (1): The ID of the target host.
*   `password` (2): The `final_hash` computed above (bytes).
*   `my_id` (4): The client's local ID (e.g., `123456789` or `123456789@rendezvous`).
*   `my_name` (5): The client's display name.
*   `my_platform` (13): The client's OS (e.g., `macOS`, `Linux`, `Windows`).
*   `session_id` (10): A random `u64` session identifier.
*   `version` (11): The client version string (e.g., `1.3.7`).
*   `os_login` (12): Optional `OSLogin` sub-message for system-level authentication.
*   `hwid` (14): Hardware ID (bytes) for "trusted device" features.

---

## 10. Rendezvous Implementation Details

The rendezvous protocol facilitates peer discovery and connectivity through the `hbbs` (ID/Rendezvous) server.

### UDP Message Sequence

#### 1. Registration (`RegisterPeer`)
To stay online and discoverable, the host periodically sends a `RegisterPeer` message to `hbbs`.
*   **Sequence**:
    1.  **Host → `hbbs` (`RegisterPk`)**: (Initial) Registers the public key and UUID.
    2.  **`hbbs` → Host (`RegisterPkResponse`)**: Confirms registration and provides `keep_alive` interval.
    3.  **Host → `hbbs` (`RegisterPeer`)**: (Periodic) Updates online status.
    4.  **`hbbs` → Host (`RegisterPeerResponse`)**: Acknowledges status.

#### 2. Connection Request (`PunchHoleRequest`)
The client initiates a connection by asking `hbbs` to help "punch a hole" to the target host.
*   **Sequence**:
    1.  **Client → `hbbs` (`PunchHoleRequest`)**: Contains `id` (target), `nat_type`, and `udp_port`.
    2.  **`hbbs` → Target (`PunchHole`)**: Forwards the request to the host.
    3.  **Target → `hbbs` (`PunchHoleSent`)**: Host acknowledges and starts punching.
    4.  **`hbbs` → Client (`PunchHoleResponse`)**: Provides target's IP, PK, and relay info.

### Building `RendezvousMessage` Protobuf

The client uses the `prost` generated `RendezvousMessage` struct. Messages are typically wrapped in a `oneof` union.

```rust
// Example: Building a RegisterPeer message
let mut msg_out = RendezvousMessage::new();
msg_out.set_register_peer(RegisterPeer {
    id: Config::get_id(),
    serial: Config::get_serial(),
    ..Default::default()
});

// Example: Building a PunchHoleRequest
let mut msg_out = RendezvousMessage::new();
msg_out.set_punch_hole_request(PunchHoleRequest {
    id: target_id.to_owned(),
    nat_type: my_nat_type.into(),
    licence_key: key.to_owned(),
    conn_type: conn_type.into(),
    udp_port: my_udp_port as _,
    ..Default::default()
});
```

### NAT Type Detection Logic
RustDesk determines NAT type by comparing the public ports returned by two different rendezvous servers (or different ports on the same server).

1.  **Request**: Client sends `TestNatRequest` to `Server1:Port1` and `Server2:Port2`.
2.  **Response**: Each server returns the client's public port in a `TestNatResponse`.
3.  **Logic**:
    *   If `Port1 == Port2`, the NAT is **ASYMMETRIC** (usually "Full Cone" or "Address Restricted").
    *   If `Port1 != Port2`, the NAT is **SYMMETRIC**.

```rust
// Simplified Logic from test_nat_type_()
let ok = port1 > 0 && port2 > 0;
if ok {
    let t = if port1 == port2 {
        NatType::ASYMMETRIC
    } else {
        NatType::SYMMETRIC
    };
    Config::set_nat_type(t as _);
}
```

### Relay Fallback Trigger Conditions
A session falls back to the relay server (`hbbr`) in the following scenarios:
1.  **Symmetric NAT on both sides**: Hole punching is mathematically unlikely to succeed.
2.  **Force Relay**: The `force_relay` flag is set in the `PunchHoleRequest`.
3.  **Direct P2P Timeout**: The client fails to establish a direct connection within the `CONNECT_TIMEOUT` after hole punching.
4.  **WebSocket/Proxy**: If the client is using a WebSocket or a SOCKS5 proxy, direct UDP P2P is often bypassed in favor of relay.

### Socket_client Functions Used
*   **`connect_tcp`**: Establishes a signaling connection to `hbbs`.
*   **`new_direct_udp_for`**: Creates a UDP socket for registration and hole punching.
*   **`connect_tcp_local`**: Used during TCP hole punching to attempt a connection from a specific local port.
*   **`rebind_udp_for`**: Re-establishes a UDP socket if the network environment changes.

---

## 11. Video Decoding for Screenshots

Capturing a screenshot from a RustDesk video stream requires decoding the incoming compressed frames and converting them into a standard image format like PNG.

### VP9 Decoding (libvpx)
RustDesk utilizes the **`libvpx`** library for decoding VP9 video streams.
*   **Initialization**: The decoder is initialized using `vpx_codec_vp9_dx()`.
*   **Decoding Process**: The `VpxDecoder::decode` method passes raw bytes from a `VideoFrame` Protobuf message into the `vpx_codec_decode` function.
*   **Frame Extraction**: Decoding a single packet may yield multiple frames. RustDesk uses an iterator (`DecodeFrames`) that wraps `vpx_codec_get_frame` to retrieve `Image` objects. For a single screenshot, the client typically iterates through all frames in the current packet and retains only the **last frame** to ensure it has the most up-to-date visual state.

### Pixel Format Conversion
The raw output from the VP9 decoder is typically in a YUV format (e.g., **I420** or **I444**), which must be converted to **RGBA** for PNG generation.
*   **Conversion Mechanism**: Conversion is handled by a `to` method on the `Image` object (e.g., `last_frame.to(rgb_buffer)`).
*   **Libraries**: RustDesk leverages **`libyuv`** (via the `vpxcodec.rs` and `codec.rs` implementations) for high-performance YUV-to-RGB conversion. This handles the necessary color space transformations and chroma upsampling.

### Generating a PNG Frame
To save a decoded frame as a PNG in the CLI client:
1.  **Buffer Allocation**: Allocate a buffer of size `width * height * 4` for the RGBA data.
2.  **Conversion**: Call the decoder's conversion routine to fill this buffer from the `last_frame` decoded from the stream.
3.  **Encoding**: Use a Rust crate like `png` or `image` to encode the raw RGBA buffer into a PNG file.
    *   **Note**: Since RustDesk is "P2P-first," the client must wait for a **Keyframe** (I-frame) before it can successfully decode and display the first image. Subsequent delta frames require the previous state.

