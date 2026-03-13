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

## 11. API Server Endpoints

The RustDesk API server (typically part of the Pro version or custom sidecars like `rustdesk-api`) facilitates client authentication, configuration, and address book management. Based on probing the test server at `http://115.238.185.55:50074`, here are the findings:

### Core Endpoints & Methods
The API server primarily uses **POST** for state-changing or sensitive actions and **GET** for configuration retrieval.

| Endpoint | Method | Observed Response | Description |
| :--- | :--- | :--- | :--- |
| `/api/login` | **POST** | `{"error": "请求方式错误！请使用POST方式。"}` | Authenticates user (ID/Password). Returns JWT token. |
| `/api/server-config` | **GET** | `404 Not Found` | Retrieves `id_server`, `relay_server`, and `key`. |
| `/api/peers` | **GET** | `{"code": 1, "data": "ok"}` | Address book management. |
| `/api/heartbeat` | **POST** | `500 Server Error` | Reports online status. |
| `/api/currentUser` | **GET** | `{"error": "错误的提交方式！"}` | Returns user profile. |
| `/api/audit` | **POST** | `500 Server Error` | Logs connection events (Pro feature). |

### Key Observations from Probes
1.  **Strict HTTP Methods**: The server explicitly enforces `POST` for `/api/login` and `/api/currentUser`. Accessing them via `GET` returns a descriptive error message.
2.  **hbbs Integration**: While `hbbs` handles the rendezvous (port 21116), the API server (port 50074 in this config) handles the business logic.
3.  **Status 500/404**: The `500 Server Error` on `/api/heartbeat` and `/api/audit` suggests these may require specific headers (like `Authorization`) or are disabled in the current configuration.

### Implementation for CLI
To support "Zero Config" in the CLI:
1.  **Login**: Use `POST /api/login` with JSON payload `{username, password, id, uuid, device_name}`.
2.  **Config**: Attempt to fetch `server-config` to auto-populate the ID and Relay server addresses.

---

## 12. Terminal Channel Protocol

RustDesk features a native terminal channel that provides a remote shell (PTY) without the overhead of video encoding. This is the preferred channel for text-based CLI interactions.

### PTY Spawning and Management
The server-side logic is implemented in `src/server/terminal_service.rs` and utilizes the **`portable-pty`** crate for cross-platform support.
- **Unix (Linux/macOS)**: Uses standard `/dev/ptmx` via `openpty`. On macOS, it defaults to a login shell (`-l`).
- **Windows**: Uses **ConPTY** (Windows 10+). Due to permission constraints, it often employs a "helper process" pattern where a separate process is launched as the logged-in user to manage the PTY, communicating with the main service via named pipes.
- **Shell Selection**: The server typically defaults to `/bin/bash` or `/bin/sh` on Unix and `powershell.exe` or `cmd.exe` on Windows.

### TerminalData Encoding and Compression
- **Bidirectional Stream**: `TerminalData` messages carry stdin (client → server) and stdout/stderr (server → client) bytes.
- **Compression**: Data is compressed using **zstd** if the payload exceeds **512 bytes**.
- **Optimization**: The server checks if the compressed data is actually smaller than the raw bytes; if not, it sends the raw data. The `compressed` boolean flag in the `TerminalData` message indicates the state.

### Sequencing and Handshake
1.  **Authentication**: The client must complete the NaCl handshake and `LoginRequest` first.
2.  **Open Request**: After receiving `LoginResponse`, the client sends `TerminalAction::OpenTerminal`.
3.  **Persistence**: If the client sends `OptionMessage` with `terminal_persistent: Yes` before opening, the server will attempt to reconnect to an existing PTY session if available.
4.  **Redraw Trigger**: Upon reconnection, the server performs a "two-phase SIGWINCH" (resizing the terminal by ±1 row and then back) to force TUI applications (like `htop` or `vim`) to redraw the screen.

### Rows, Cols, and Resizing
- **Initialization**: `OpenTerminal` includes the initial `rows` and `cols`.
- **Dynamic Resize**: `TerminalAction::ResizeTerminal` updates the PTY size at runtime. The server handles this by calling the underlying PTY's `resize` method or sending a resize control message to the Windows helper process.

### Permissions
Terminal access requires the **`terminal`** permission flag. While not explicitly checked in `terminal_service.rs`, the main message dispatcher verifies that the session has the `ControlPermissions::Permission::terminal` bit set before forwarding terminal actions.

---

## 13. Terminal Protocol Optimizations

Efficient terminal communication is achieved through data compression, multi-session management, and flow control mechanisms.

### Data Compression (zstd)
RustDesk uses the **zstd** algorithm for real-time compression of text-heavy payloads.
- **Threshold**: Payload compression is triggered when the data size exceeds **512–1024 bytes**. Small chunks are sent raw to avoid the CPU and header overhead of compression.
- **Toggling**: The `compressed` boolean flag in `TerminalData` or `Clipboard` messages indicates whether the payload requires decompression at the destination.

### Multiple Terminal Sessions
The protocol supports multiple concurrent shells through the **`terminal_id`** field.
- **Server-side Mapping**: The host maintains a `HashMap<i32, Session>` to route data to the correct PTY.
- **ID Management**: The client assigns IDs (starting from 0). Upon reconnection, the client can request to remap existing persistent sessions to new IDs.

### Session Persistence (`terminal_persistent`)
The `OptionMessage` includes a `terminal_persistent` flag that controls the lifecycle of terminal sessions.
- **Behavior**: If enabled, terminal processes (PTYs) are not killed when the network connection drops. They are stored in a global registry on the host.
- **Reconnection**: A client can reconnect to these "orphaned" sessions by sending an `OpenTerminal` request with the same parameters, allowing for continuity in long-running tasks.

### Clipboard Protocol Flow (`cliprdr`)
RustDesk implements a variant of the RDP **`cliprdr`** virtual channel for clipboard synchronization.
- **Advertisement**: When the local clipboard changes, the client sends a `Cliprdr::format_list` containing available formats (Text, HTML, Image, etc.).
- **Synchronization**: Every message includes a **sequence number** to ensure the most recent "Copy" event takes precedence.
- **Data Pull**: The actual content is typically pulled on-demand (when the peer attempts a "Paste") using `format_data_request`.

### Keystroke Batching
- **Default**: Most interactive keystrokes are sent as individual `KeyEvent` messages (down/up pairs) to minimize latency.
- **Batching (`seq`)**: The `KeyEvent` message includes a **`seq`** string field. This allows the client to send entire strings (e.g., automated commands or passwords) in a single packet, which the server then injects into the PTY input buffer.

### Flow Control and Backpressure
- **Bounded Channels**: The server uses bounded synchronous channels (typically size 500) for terminal output.
- **Data Dropping**: If the client is too slow to consume output (e.g., during a `cat /dev/urandom` spike), the server will **drop data chunks** rather than deadlocking or exhausting memory. This protects the host's stability.

### Connection Type (`ConnType`) Implications
- **`TERMINAL` type**: If the connection is established specifically for a terminal session, the host **disables the video scraper** and encoder, significantly reducing CPU and bandwidth usage.
- **`DEFAULT_CONN` type**: If a terminal is opened within a desktop session, the video stream remains active. For the CLI client, it is highly recommended to use the terminal-specific connection type when desktop visuals are not required.

---

## 14. hbbr Relay Handshake Details

The `hbbr` relay server acts as a matchmaker for peers that cannot establish a direct P2P connection. The initial TCP handshake and binding process are critical for session establishment and stability.

### Initial Protobuf Message
Immediately after establishing a TCP connection to `hbbr` (default port 21117), the client must send a `RendezvousMessage` containing the `RequestRelay` variant.
- **Message**: `RendezvousMessage { union: Some(Union::RequestRelay(RequestRelay { ... })) }`
- **Fields**:
  - `id`: The RustDesk ID of the target peer.
  - `uuid`: The unique session identifier (GUID) provided by `hbbs` during the discovery phase.
  - `token`: An authentication token generated by `hbbs` to authorize the relay session.
  - `secure`: Boolean flag. If true, the subsequent session will expect a NaCl handshake.

### uuid/token Binding Mechanism
- **uuid**: Acts as the primary lookup key. It is a transient session identifier that "glues" the two peers together.
- **token**: Used for security verification. In the OSS version, it validates that the relay request was brokered by a legitimate `hbbs` instance. In the Pro version, it may contain JWT-based access permissions.

### Peer Pairing Logic
`hbbr` maintains a global in-memory map of waiting connections indexed by `uuid`.
1.  **Arrival**: When a peer connects and sends `RequestRelay`, `hbbr` checks if a connection with the same `uuid` already exists in the map.
2.  **Waiting**: If no match is found, the current connection (socket stream) is added to the map.
3.  **Bridges**: When the second peer (with the same `uuid`) arrives, `hbbr` retrieves the waiting stream from the map and "bridges" the two sockets.
4.  **Forwarding**: Once paired, `hbbr` enters a transparent relay mode, bi-directionally forwarding raw bytes between the two peers. It does not (and cannot) decrypt the data.

### Timeouts and Keepalives
- **Binding Timeout**: `hbbr` will only wait for a matching peer for approximately **30 seconds**. If the pairing is not completed within this window, the initial connection is dropped to reclaim resources.
- **Heartbeat Requirement**: To prevent NAT/Firewall idle timeouts, the peers should send heartbeats. If no data or heartbeat is received for **10–15 seconds**, `hbbr` terminates the session.
- **Initial Handshake Grace Period**: The `RequestRelay` message must be received within a few seconds of the TCP connection establishment, or the server will close the socket.

---

## 15. Video Decoding for Screenshots

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

---

## 16. Relay Binding & Session Handshake

When a direct P2P connection fails, the client must use the relay server (`hbbr`) and perform a cryptographic handshake to establish an end-to-end encrypted (E2EE) session.

### Relay Binding Sequence (`hbbr`)
The relay server acts as a transparent bridge. The "binding" ensures that the Controller and Controlled sides are connected to the same session.

1.  **Transport**: Connect via TCP to `hbbr` (default port 21117).
2.  **Framing**: All messages are prefixed with a 4-byte little-endian length header.
3.  **Binding Message**: The client sends a `RendezvousMessage` containing a `RequestRelay` sub-message.
    *   `uuid`: The unique session ID provided by `hbbs` in the `RelayResponse`.
    *   `id`: The target peer's ID.
    *   `token`: The relay token provided by `hbbs`.
4.  **Verification**: If the `uuid` is valid and the peer has also connected, `hbbr` binds the two sockets and starts forwarding raw bytes.

### Secure Channel Handshake (NaCl Phase 2)
Once a transport (Direct or Relay) is established, the E2EE tunnel is initialized.

1.  **Identity Exchange**:
    *   **Host → Client**: Sends a `Message` containing `SignedId` (Host's ID and Ed25519 Public Key).
    *   **Verification**: The client verifies the signature using the server's long-term public key (the "Key" string from settings).
2.  **Key Exchange**:
    *   **Client**: Generates an ephemeral Curve25519 key pair and a random 32-byte symmetric **Session Key**.
    *   **Client → Host**: Sends a `Message` containing `PublicKey`.
        *   `asymmetric_value`: Client's ephemeral public key (bytes).
        *   `symmetric_value`: The Session Key encrypted using NaCl `box_seal` (Host's PK, Client's SK, zeroed nonce).
3.  **Session Encryption**:
    *   From this point forward, every `Message` is encrypted using NaCl `secretbox` with the established Session Key.
    *   **Nonce**: 24 bytes (First 8 bytes = sequence number (u64, LE), remaining 16 bytes = 0).

### Authentication (Phase 3)
1.  **Challenge**: Host sends an encrypted `Hash` message containing a `salt` and a `challenge` string.
2.  **Login**: Client sends an encrypted `LoginRequest`.
    *   `password`: `SHA256(SHA256(plaintext_pw + salt) + challenge)`.
    *   `my_id`: Client ID.
    *   `session_id`: Random `u64`.
3.  **Session Start**: Host responds with an encrypted `LoginResponse` containing `PeerInfo` (displays, resolution, features).

---

## 17. TCP Hole Punching Sequence

In environments where UDP is restricted, RustDesk attempts TCP hole punching to establish a direct P2P connection before falling back to a relay.

### Coordinated Simultaneous Open
TCP hole punching relies on both peers attempting to connect to each other at the same time using the same local port that was used to communicate with the rendezvous server (`hbbs`).

1.  **Socket Binding**: The client must bind its local TCP socket to the same local port used for its UDP registration/discovery. This requires setting `SO_REUSEADDR` and `SO_REUSEPORT` on the socket before calling `connect`.
2.  **Signaling**:
    *   **Client A → `hbbs` (`PunchHoleRequest`)**: Request a connection to Peer B.
    *   **`hbbs` → Peer B (`PunchHole`)**: Forward the request with Peer A's public address.
    *   **Peer B → `hbbs` (`PunchHoleSent`)**: Peer B starts its `connect` attempt to Peer A and notifies the server.
    *   **`hbbs` → Client A (`PunchHoleResponse`)**: Server provides Peer B's public address to Client A.
3.  **Simultaneous Connect**:
    *   Both peers call `connect()` to each other's public IP:port.
    *   If the NAT devices are endpoint-independent (Full Cone or Restricted), the outgoing SYN from Peer A will "punch" a hole that allows Peer B's incoming SYN to pass through, and vice versa.
4.  **Simultaneous Open**: The TCP stack on both sides sees a SYN followed by an incoming SYN, transitioning directly to the `ESTABLISHED` state via a "simultaneous open" handshake (SYN → SYN+ACK).

### Intranet Optimization (`FetchLocalAddr`)
If both peers are on the same local network, `hbbs` will detect this and facilitate a direct LAN connection.
1.  **`hbbs` → Peer B (`FetchLocalAddr`)**: Server asks for B's internal IP.
2.  **Peer B → `hbbs` (`LocalAddr`)**: B provides its local IP (e.g., `192.168.1.50`).
3.  **`hbbs` → Client A (`LocalAddr`)**: Server provides B's local IP to A.
4.  **Direct Connect**: Client A attempts a standard TCP connection to Peer B's local IP, bypassing NAT traversal entirely.

---

## 18. Pure-Rust NaCl Key Conversion

To maintain compatibility with the official RustDesk client (which uses `sodiumoxide`) while keeping the `rustdesk-cli` build simple and pure-Rust, we must correctly convert Ed25519 identity keys to Curve25519 (X25519) encryption keys.

### Public Key Conversion (Ed25519 → X25519)
The `ed25519-dalek` crate's `VerifyingKey` can be mapped to an `x25519-dalek` `PublicKey` using the birational map between the Edwards and Montgomery forms.

```rust
use ed25519_dalek::VerifyingKey;
use x25519_dalek::PublicKey as X25519PublicKey;

fn convert_pk(ed_pk_bytes: &[u8]) -> X25519PublicKey {
    let ed_pk = VerifyingKey::from_bytes(ed_pk_bytes.try_into().unwrap()).unwrap();
    let x_pk_bytes = ed_pk.to_montgomery().to_bytes();
    X25519PublicKey::from(x_pk_bytes)
}
```

### Secret Key Conversion (Ed25519 → X25519)
Converting the secret key (seed) requires hashing the seed with **SHA-512** and taking the first 32 bytes as the Montgomery scalar.

```rust
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;
use sha2::{Sha512, Digest};

fn convert_sk(ed_sk: &SigningKey) -> StaticSecret {
    let mut hasher = Sha512::new();
    hasher.update(ed_sk.to_bytes());
    let hash = hasher.finalize();
    
    let mut x_sk_bytes = [0u8; 32];
    x_sk_bytes.copy_from_slice(&hash[..32]);
    StaticSecret::from(x_sk_bytes)
}
```

### Encryption Compatibility
- **Handshake Box**: Use `crypto_box::Box` with the converted keys and a **zeroed nonce** (`[0u8; 24]`) to seal the symmetric session key.
- **Session Messages**: Use `xsalsa20poly1305::XSalsa20Poly1305` with the session key and a nonce containing the **little-endian sequence number** (first 8 bytes).
- **Password Hashing**: Use `sha2::Sha256` for the challenge-response hash: `SHA256(SHA256(pw + salt) + challenge)`.

---

## 19. Input Event Details

Injecting keyboard and mouse input correctly requires understanding the coordinate system and input modes.

### Mouse Events
- **Coordinate System**: RustDesk uses absolute coordinates $(x, y)$ corresponding to the remote screen resolution (e.g., $1920 \times 1080$).
- **Button Masks**:
    - `Left`: 1
    - `Right`: 2
    - `Middle`: 4
    - `Scroll Up`: 8
    - `Scroll Down`: 16
- **`is_move` Flag**: Set to `true` for pointer motion, `false` for button press/release.

### Keyboard Modes (`KeyEvent`)
- **Map Mode (`0`)**: Sends raw hardware scancodes. The remote machine interprets these based on its *own* keyboard layout. This is brittle for CLI agents.
- **Translate Mode (`2`)**: Recommended for the CLI client. The client sends the intended character or virtual key, and the host ensures that specific character is typed, regardless of layout differences.
- **Special Keys**: Use the `control_key` enum in `KeyEvent`.
    - Example: `Return`, `Escape`, `Backspace`, `Tab`, `Shift`, `Control`, `Alt`, `Meta` (Windows/Command).
    - When sending a special key, the `chr` field is typically omitted, and `control_key` is populated.

### Implementation Strategy for `rustdesk-cli`
- **`type` command**: Use `Translate` mode and send a sequence of `down: true` and `down: false` events for each character.
- **`key` command**: Use `control_key` for modifiers and special keys.
- **`click` command**: Send a `down: true` event followed immediately by a `down: false` event at the same coordinates.

---

## 20. Screenshot Capture Protocol

RustDesk provides a dedicated mechanism for on-demand screen captures via `ScreenshotRequest` and `ScreenshotResponse` messages. This is distinct from the continuous video stream.

### Message Format
- **`ScreenshotRequest`**:
    - `display` (int32): The index of the display to capture (default is 0).
    - `sid` (string): The session ID of the controlling side, used for routing the response.
- **`ScreenshotResponse`**:
    - `sid` (string): The session ID to match the request.
    - `msg` (string): An error message; empty if the capture was successful.
    - `data` (bytes): The encoded image data.

### Data Encoding
The `data` field in `ScreenshotResponse` contains a complete, self-describing image file.
- **Format**: Usually **PNG** (lossless) or **JPEG** (lossy), depending on the server's internal configuration. Since the protocol lacks a explicit `format` field, the client should use an image library (like the Rust `image` crate) that can detect the format from magic bytes (headers).
- **Capture Source**: The server uses the **`scrap`** library to capture the frame buffer directly from the OS (DXGI on Windows, X11/Wayland on Linux).

### Stream Independence
- **Stand-alone Operation**: `ScreenshotRequest` does **not** require an active video stream. It triggers a one-off capture and encoding process on the server.
- **TERMINAL ConnType**: Even if the connection is established with `TERMINAL` connection type (which disables the continuous video scraper), the `ScreenshotRequest` handler remains functional as it explicitly invokes the capture logic.
- **Efficiency**: This is the preferred method for CLI agents to "see" the remote screen without the bandwidth and CPU overhead of decoding a live VP9/H264 stream.

### Latency and Performance
- **Capture + Encode**: Server-side processing (capturing raw pixels and encoding to PNG/JPEG) typically takes **50ms–150ms**.
- **Transmission**: The response size depends on screen resolution and complexity (e.g., a 1920x1080 PNG can be 200KB–2MB).
- **Round-trip**: On a standard broadband connection, the total latency from request to receiving the full PNG is expected to be **200ms–600ms**.

### Alternative: Video Stream Snapping
If `ScreenshotRequest` is unsupported by a specific host version, the client can fall back to:
1.  Enabling the video stream.
2.  Waiting for the first **Keyframe** (I-frame).
3.  Decoding that single frame using `libvpx` (VP9) or `openh264`.
4.  Saving the resulting RGBA buffer as a PNG.
---

## 21. Zstd Implementation Guide

This guide provides a practical roadmap for implementing zstd compression in the terminal and clipboard channels to reduce bandwidth usage.

### 1. Cargo.toml Changes
Add the `zstd` crate to the `[dependencies]` section:
```toml
zstd = "0.13"
```

### 2. API Usage
Use the high-level `encode_all` and `decode_all` functions for simple `Vec<u8>` transformations.

- **Compression**:
  ```rust
  // Level 0 uses the default (typically 3)
  let compressed_data = zstd::encode_all(&raw_data[..], 0)?;
  ```
- **Decompression**:
  ```rust
  let decompressed_data = zstd::decode_all(&compressed_data[..])?;
  ```

### 3. Recommended Threshold
To balance CPU overhead and bandwidth savings, only compress payloads that exceed a specific size.
- **Threshold**: **1024 bytes**.
- **Logic**: If `raw_data.len() < 1024`, send raw bytes and set `compressed: false`. Otherwise, compress and set `compressed: true`.

### 4. Integration with TerminalData
When sending or receiving `TerminalData` in `src/terminal.rs`:

**Sending (Client → Host):**
1. Check if input string/bytes exceed the threshold.
2. If yes, compress the data.
3. Populate `TerminalData { data: compressed_bytes, compressed: true, .. }`.

**Receiving (Host → Client):**
1. Check the `compressed` field in the incoming `TerminalData` message.
2. If `true`, call `zstd::decode_all(&msg.data)`.
3. If `false`, use `msg.data` directly as raw bytes.

### 5. Error Handling
Always handle decompression errors gracefully. If `decode_all` fails (e.g., due to a corrupted packet), the session should log the error and potentially reconnect to ensure terminal state consistency.

