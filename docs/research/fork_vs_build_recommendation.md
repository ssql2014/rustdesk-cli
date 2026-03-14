# Final Recommendation: Fork vs. Build From Scratch

This document provides a strategic recommendation for the `rustdesk-cli` project trajectory.

## 1. Fork + Strip GUI (Official RustDesk)

**Effort Estimate:** Very High / Prohibitive
**Blockers Found:**
- **Unconditional Dependencies:** Even with `--features cli --no-default-features`, the build fails due to hard-coded dependencies on `scrap` (screen capture) and its underlying C++ libraries.
- **External Toolchain Requirements:** Requires `vcpkg`, `libyuv`, `libvpx`, `opus`, and `aom`. These are complex to manage across target environments (especially AI-agent nodes).
- **GUI-Centric Architecture:** The codebase is deeply coupled. Stripping the GUI is not a configuration task; it is a major architectural refactoring of `Cargo.toml` and `src/server/`.
- **Resource Footprint:** A full build exceeds 4GB of disk space and produces heavy, dynamic-linked binaries.

## 2. From-Scratch (Current `rustdesk-cli`)

**Current Status:**
- **Handshake & Auth:** Fully implemented and verified.
- **Rendezvous Protocol:** Working. Registration and peer discovery are functional.
- **"Offline" Bug Resolution:** Research confirmed that "Offline" was often a server-side license mismatch or a misunderstanding of the success-state (where the server sends NO response to the requester).
**What's Left:**
- **TCP Relay Flow:** Transitioning `RequestRelay` from UDP to TCP to match the official server's protocol.
- **Terminal Session Logic:** Implementing the UDS (Unix Domain Socket) and shell streaming protocol defined in our design docs.
- **Refining Text Transport:** Optimizing the text-only session layer for AI agents.

## Final Recommendation: Continue From-Scratch Build

**Decision: CONTINUE FROM-SCRATCH**

### Rationale:
1. **Portability:** Our pure-Rust implementation builds in seconds, has zero external C++ dependencies, and produces a single static binary. This is critical for the target use case (CLI/Headless).
2. **Protocol Mastery:** We have already navigated the hardest part (NaCl handshake). Implementing the TCP relay flow is a minor protocol adjustment compared to fighting the official repo's build system.
3. **Dead Weight:** The official repo carries thousands of lines of code for video codecs, rendering, and cross-platform UI that will never be used in a CLI tool.
4. **Agility:** Maintaining a fork of a fast-moving, 100k+ LOC project is a long-term maintenance burden. A focused CLI tool is much easier to keep compatible.

**Next Action:** Pivot to implementing the **TCP-based RequestRelay flow** to achieve full compatibility with official RustDesk servers.
