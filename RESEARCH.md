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
4.  **`hbbr` (Binding)**: The relay server bridges the two TCP streams.
5.  **E2EE Tunnel**: The parties perform the same **Phase 2 (NaCl Handshake)** over the relay. `hbbr` only sees encrypted traffic and cannot decrypt it.
