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
