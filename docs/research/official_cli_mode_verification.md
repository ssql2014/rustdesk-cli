# Verification: Official RustDesk CLI Mode on macOS

**Date:** 2026-03-14
**Verified by:** Max (DV/QA)
**Repo:** https://github.com/rustdesk/rustdesk (cloned to /tmp/rustdesk-verify)

## 1. Feature Flag Analysis

The `cli` feature is declared in `Cargo.toml` (line 25):

```toml
[features]
cli = []
default = ["use_dasp"]
```

It's an **empty feature** — it doesn't pull in or exclude any optional dependencies at the
Cargo level. Its effect is entirely compile-time via `#[cfg(feature = "cli")]` gates in source.

### What `cli` excludes via cfg gates

| Module | Gate | Effect |
|--------|------|--------|
| `ui` (Sciter GUI) | `#[cfg(not(any(..., feature = "cli")))]` | **Excluded** — Sciter UI not compiled |
| `core_main` | `#[cfg(not(any(..., feature = "cli")))]` | **Excluded** — no GUI startup logic |
| `CUR_SESSION` (Sciter) | `#[cfg(not(any(feature = "flutter", feature = "cli")))]` | **Excluded** |
| `keyboard` Sciter paths | `#[cfg(not(any(feature = "flutter", feature = "cli")))]` | **Excluded** |
| `file_trait` Sciter paths | multiple cfgs | **Excluded** |

### What `cli` does NOT exclude (still compiled)

| Module/Dep | Why it stays |
|------------|-------------|
| `server` module | No cli gate — always compiled on macOS |
| `platform` module | Always compiled (macOS-specific code) |
| `tray` module | Only gated on android/ios |
| `whiteboard` module | Only gated on android/ios |
| `clipboard` module | Only gated on ios |
| `rendezvous_mediator` | Only gated on ios |
| `ui_interface` | Always compiled |
| `ui_session_interface` | Always compiled |
| `ui_cm_interface` | Always compiled |

## 2. CLI Entry Point

With `--features cli`, `src/main.rs` uses a dedicated `main()` (line 36-104):

```rust
#[cfg(feature = "cli")]
fn main() {
    // Uses clap for arg parsing
    // Supports: --port-forward, --connect, --server, --key
}
```

### Available CLI Commands

| Flag | Function | Description |
|------|----------|-------------|
| `-p, --port-forward` | `cli::start_one_port_forward()` | Format: `remote-id:local-port:remote-port[:remote-host]` |
| `-c, --connect` | `cli::connect_test()` | Test connection (PORT_FORWARD ConnType only) |
| `-k, --key` | (parameter) | Server key |
| `-s, --server` | `start_server()` | Start as server/daemon |

### CLI Session (`src/cli.rs`)

- 194 lines, implements `Interface` trait
- Hardcoded to `ConnType::PORT_FORWARD` only
- Uses `rpassword` for terminal password input
- No remote desktop viewing, file transfer, or terminal session support
- Marked as "test only" for `--connect`

## 3. Build Feasibility: `cargo build --features cli --no-default-features`

### Verdict: **WILL NOT BUILD without system dependencies**

Even though the Sciter UI module is excluded, **heavy system dependencies remain unconditionally**:

#### macOS-Specific Hard Dependencies (from Cargo.toml)

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
cocoa = "0.24"              # macOS AppKit bindings
dispatch = "0.2"            # GCD
core-foundation = "0.9"
core-graphics = "0.22"      # Screen capture
fruitbasket = "0.10"        # macOS app lifecycle
piet = "0.6"                # 2D graphics
piet-coregraphics = "0.6"   # CoreGraphics renderer
```

#### Desktop Hard Dependencies (not gated on cli)

```toml
[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]
sciter-rs = { git = "..." }     # Sciter runtime (linked even if ui module excluded!)
enigo = { path = "libs/enigo" } # Input simulation
clipboard = { path = "..." }    # Clipboard access
arboard = { git = "..." }       # Clipboard
clipboard-master = { git = "..." }
portable-pty = { git = "..." }  # PTY for terminal sessions
tray-icon = { git = "..." }     # System tray
tao = { git = "..." }           # Window management
image = "0.24"                  # Image processing
```

#### Other Heavy Dependencies (always compiled)

```toml
scrap = { path = "libs/scrap" }  # Screen capture library
magnum-opus = { git = "..." }    # Audio codec (requires C build)
cpal = { git = "..." }           # Audio I/O
rdev = { git = "..." }           # Input device events
```

#### build.rs Always Links macOS Frameworks

```rust
// build.rs line 88-91 — unconditional on macOS
if target_os == "macos" {
    build_mac();  // Compiles macos.mm (requires Xcode/AppKit headers)
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
}
```

### Key Problem: Sciter-rs

Even though `pub mod ui` is excluded by `#[cfg(not(... feature = "cli"))]`, the
**sciter-rs crate is still listed as a hard dependency** for all desktop targets.
Cargo resolves and compiles it regardless of whether any code imports it. The crate
itself requires the Sciter SDK dynamic library at link time.

## 4. Summary

| Question | Answer |
|----------|--------|
| Does a `cli` feature exist? | **Yes**, but it's minimal |
| Does it exclude GUI modules? | **Yes** — `ui` (Sciter), `core_main` excluded |
| Does `--no-default-features` help? | **Slightly** — drops `use_dasp` audio resampling only |
| Can it build without Xcode? | **No** — build.rs compiles `macos.mm`, links AppKit |
| Can it build without Sciter SDK? | **No** — sciter-rs is an unconditional desktop dep |
| Is the CLI useful? | **Barely** — only port-forward and a test connect |
| Does it support remote desktop? | **No** |
| Does it support file transfer? | **No** |
| Does it support terminal? | **No** (despite `--terminal` existing in Flutter mode) |

## 5. Implications for rustdesk-cli

The official RustDesk `cli` feature is:
1. **Not a true headless CLI** — it still requires all GUI/system dependencies to compile
2. **Extremely limited** — only port-forward with a "test" connect mode
3. **Not maintained as a first-class target** — the feature flag only gates source modules,
   not Cargo dependencies

This confirms the design decision behind `rustdesk-cli`: building a separate, clean CLI
client from scratch using only the protocol/crypto primitives is the correct approach.
Trying to strip the official codebase down to CLI-only would require:
- Refactoring ~20 unconditional dependency declarations into optional/feature-gated ones
- Splitting the server module out of the main crate
- Removing or stubbing cocoa/AppKit/CoreGraphics/Sciter dependencies
- Essentially rewriting the build system

Our approach (standalone crate with targeted deps: `hbb_common` protos, `sha2`, `crypto_box`,
`ed25519-dalek`, `tokio`) avoids all of this complexity.
