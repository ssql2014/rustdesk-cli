# DESIGN_TEXT_SESSION

## Purpose

Define the text-mode session architecture for `rustdesk-cli` so that:

- the daemon owns a real RustDesk text connection instead of stubbed session state
- `shell` uses a true interactive terminal lifecycle
- `exec` runs a remote command and returns its output deterministically
- clipboard text is synchronized over the RustDesk protobuf protocol
- timeouts, disconnects, and partial terminal reads are handled predictably

This document is design only. It does not prescribe implementation details beyond module responsibilities, state transitions, and wire-level behavior.

## Scope

In scope:

- wiring `src/text_session.rs` through `src/daemon.rs`
- session lifecycle for connect, shell, exec, clipboard, and disconnect
- protobuf messages required for terminal and clipboard
- daemon-side error handling and runtime ownership
- module dependency graph

Out of scope:

- GUI input/video path
- file transfer
- prompt rendering details in the local CLI
- exhaustive retry logic for live RustDesk protocol edge cases

## Current State

The current codebase has the right pieces, but they are not yet composed correctly:

- [`src/connection.rs`](/Users/qlss/Documents/Projects/rustdesk-cli/src/connection.rs) already performs the real rendezvous, relay, crypto, and login flow.
- [`src/text_session.rs`](/Users/qlss/Documents/Projects/rustdesk-cli/src/text_session.rs) already models a text-capable RustDesk session and sends `OptionMessage` plus terminal open/data/close operations.
- [`src/terminal.rs`](/Users/qlss/Documents/Projects/rustdesk-cli/src/terminal.rs) already wraps `TerminalAction` and `TerminalResponse`.
- [`src/daemon.rs`](/Users/qlss/Documents/Projects/rustdesk-cli/src/daemon.rs) still treats connect as a stubbed state transition and uses a one-shot JSON-over-UDS request/response pattern.
- [`src/session.rs`](/Users/qlss/Documents/Projects/rustdesk-cli/src/session.rs) is currently a logical command model, not a holder of live network resources.

The main architectural gap is that the daemon currently has no owner for the live encrypted RustDesk stream. That ownership must move into daemon runtime state.

## Design Principles

1. The daemon owns all live remote resources.
2. `Session` remains control-plane state for CLI-visible status and command validation.
3. `text_session` owns the RustDesk text transport and terminal lifecycle.
4. Only one component may read from the encrypted RustDesk stream.
5. Terminal output is a byte stream, not a line stream.
6. Clipboard reception is event-driven, not request/response driven.
7. Interactive shell and one-shot exec must not interfere with each other.

## 1. Daemon Wiring

### 1.1 Runtime Ownership Model

The daemon should own a runtime object with two layers of state:

- control state: external session status exposed to the CLI
- runtime state: live RustDesk connection, inbound router, clipboard cache, and optional active terminal

Recommended daemon-owned runtime model:

- `Session`
  - coarse connection state visible to CLI
  - connection metadata such as peer id and peer info
- `TextSession`
  - live encrypted RustDesk transport
  - peer info returned by login
  - terminal-related runtime handles
- `TerminalRuntime` optional
  - active terminal id
  - terminal service id
  - terminal mode: `Interactive` or `Exec`
- `ClipboardCache`
  - latest inbound text clipboard
  - timestamp
  - source format
- `InboundRouter`
  - the only task allowed to read from the encrypted RustDesk stream

`Session` should not own `EncryptedStream` directly. It is a serializable control-plane structure. The daemon runtime should own the non-serializable live transport.

### 1.2 Real Connect Flow In The Daemon

When the daemon starts, it should replace the current stubbed `SessionCommand::Connect` dispatch with a real text connection flow:

1. Build a `connection::ConnectionConfig` from daemon startup arguments.
2. Call the text-session connect path instead of `session.dispatch(Connect)` for network establishment.
3. On success:
   - store the returned text connection in daemon runtime state
   - copy peer metadata into `Session.peer_info`
   - set `Session.state = Connected`
   - initialize clipboard sync policy and terminal availability
4. On failure:
   - set `Session.state = Disconnected`
   - write an error response to the spawning CLI path
   - remove lock and socket

### 1.3 Recommended `text_session` Refactor

The current `text_connect()` opens a terminal immediately. The daemon needs finer control than that.

The desired logical split is:

- connection establishment
  - rendezvous, relay, crypto, login
  - send `OptionMessage`
  - return a connected text-capable session with no active terminal yet
- terminal open
  - open a terminal on demand for `shell` or `exec`
- terminal close
  - close the active terminal without tearing down the whole RustDesk connection
- transport disconnect
  - close the encrypted stream and fully disconnect

Design intent:

- `connect` establishes the remote RustDesk session
- `shell` opens a terminal if one is not already open
- `exec` opens an ephemeral terminal, uses it, then closes it
- `disconnect` closes any terminal and then closes the transport

### 1.4 Single Inbound Reader Requirement

This is the critical daemon design decision.

Today, `terminal.rs` reads directly from the encrypted stream and ignores unrelated message types until it finds a terminal response. That is acceptable for isolated terminal tests, but not for a real daemon because the same stream may carry:

- `TerminalResponse`
- `Clipboard`
- `MultiClipboards`
- `PeerInfo`
- `Misc`
- future permission or notification messages

The daemon therefore needs a single inbound router task that:

1. reads every `Message` from the encrypted stream
2. decodes the protobuf union
3. dispatches each inbound message by type

Routing responsibilities:

- `TerminalResponse` -> active terminal channel keyed by `terminal_id`
- `Clipboard` -> clipboard cache update
- `MultiClipboards` -> clipboard cache update
- `PeerInfo` -> session metadata refresh
- terminal-independent errors -> daemon runtime error state
- unknown or unsupported messages -> debug log and ignore

Without this router, clipboard traffic can be lost or terminal readers can consume messages meant for other subsystems.

### 1.5 Outbound Access Model

All outbound writes to the encrypted stream should be serialized through a single writer handle. The daemon may have multiple logical operations, but only one write path should talk to the socket at a time.

Recommended model:

- daemon runtime holds a write-side mutex or command queue
- terminal actions, clipboard sends, and option updates all use the same writer path

## 2. Session Lifecycle

### 2.1 High-Level States

Externally visible session states may remain simple:

- `Disconnected`
- `Connecting`
- `Connected`

Internally, the daemon should track finer-grained runtime substates:

- `NoTransport`
- `ConnectingTransport`
- `ConnectedIdle`
- `OpeningTerminal`
- `InteractiveShellActive`
- `ExecActive`
- `ClosingTerminal`
- `Disconnecting`
- `Broken`

### 2.2 Connect Lifecycle

`connect` should perform:

1. rendezvous discovery
2. relay connect
3. crypto handshake
4. login/auth
5. send `Misc::Option(OptionMessage)`
6. start inbound router
7. transition to `ConnectedIdle`

`OptionMessage` policy for text mode:

- `disable_audio = Yes`
- `disable_camera = Yes`
- `image_quality = Low`
- `terminal_persistent = Yes`
- `disable_clipboard = No`

`disable_clipboard = No` is preferred over `NotSet` because the CLI is intentionally opting into clipboard synchronization.

### 2.3 Shell Lifecycle

Desired shell lifecycle:

1. CLI sends `Shell` attach request to daemon
2. daemon verifies session is connected and no conflicting terminal operation exists
3. daemon opens a terminal with `OpenTerminal`
4. daemon transitions to `InteractiveShellActive`
5. daemon and CLI enter bidirectional streaming mode
6. stdin bytes from CLI are forwarded as `TerminalAction::Data`
7. stdout/stderr bytes from remote are forwarded back to CLI
8. local resize events become `TerminalAction::Resize`
9. user exit, client disconnect, or daemon shutdown sends `TerminalAction::Close`
10. daemon waits for `TerminalResponse::Closed` or a close timeout
11. daemon returns to `ConnectedIdle`

### 2.4 Interactive UDS Protocol

The existing one-shot JSON-line UDS protocol is sufficient for connect, exec, status, and clipboard. It is not sufficient for interactive shell.

For `shell`, the UDS connection should be upgraded into a long-lived bidirectional stream after the initial command acknowledgement.

Recommended shell attach flow:

1. CLI opens the daemon UDS socket.
2. CLI sends a single JSON command line indicating `Shell`.
3. Daemon replies with a single JSON acknowledgement line.
4. If successful, both sides switch the same Unix stream into framed shell mode.

Framed shell mode should carry four logical message classes:

- input bytes
- output bytes
- resize events
- close/error events

The design requirement is framing, not a specific binary format. The format must preserve raw bytes without newline semantics.

### 2.5 Disconnect Lifecycle

`disconnect` should:

1. close any active terminal first
2. stop accepting new shell or exec operations
3. stop the inbound router
4. close the encrypted RustDesk transport
5. clear clipboard cache and terminal runtime state
6. set `Session.state = Disconnected`
7. remove daemon socket and lock

## 3. Exec Command Design

## 3.1 Core Behavior

`exec` should not reuse an interactive shell terminal. It should use a dedicated ephemeral terminal session so that:

- the output belongs only to that command
- prompt state is deterministic
- user interactive shell state is not polluted
- the daemon can enforce single-command completion semantics

Recommended rule:

- if `InteractiveShellActive`, `exec` returns a busy error
- otherwise `exec` opens its own temporary terminal

### 3.2 Exec Lifecycle

1. verify connected state and no interactive shell is active
2. open terminal with `OpenTerminal`
3. wait for the initial prompt or initial terminal quiet period
4. send command bytes
5. read terminal output until command completion boundary
6. close the terminal
7. return exit metadata and captured bytes in `SessionResponse`

### 3.3 Completion Boundary

The daemon should not rely on visually parsing the remote prompt as a generic shell feature. Prompts are user-configurable, dynamic, colored, and may contain working directory, git status, or timestamps.

The recommended completion boundary is a daemon-generated sentinel:

- generate a unique marker per exec request
- send the user command followed by a shell fragment that prints the marker and exit status
- read terminal bytes until that marker is observed
- strip the marker line from returned output
- parse exit status from the marker payload

Rationale:

- deterministic across prompts
- independent of shell theme
- robust against partial reads
- avoids indefinite waits when prompt text changes

The document request says "read output until prompt". In practice, the daemon should treat the sentinel as the prompt boundary surrogate. The daemon may still capture the initial prompt during terminal open for diagnostics, but execution completion must be sentinel-driven.

### 3.4 Exec Output Model

The exec result returned over the normal daemon response channel should include:

- raw collected bytes
- lossy UTF-8 rendering for CLI JSON/text output
- parsed remote exit code
- whether the terminal closed unexpectedly
- timeout metadata if the sentinel was not seen

### 3.5 Exec Timeouts

Exec requires two timeouts:

- startup timeout
  - max time to receive the initial terminal-open response
- completion timeout
  - max time to observe the sentinel after sending the command

An additional idle timeout may be used only as a fallback diagnostic, not as the primary completion detector.

## 4. Clipboard Protocol Design

### 4.1 Protobuf Messages

For plain text clipboard sync, use the RustDesk `Message` union entries:

- `Message::Clipboard`
- `Message::MultiClipboards`

Relevant message types from `proto/message.proto`:

- `Clipboard`
  - `compress`
  - `content`
  - `width`
  - `height`
  - `format`
  - `special_name`
- `MultiClipboards`
  - repeated `Clipboard`

Relevant formats:

- `ClipboardFormat::Text`

Not used for plain text CLI clipboard:

- `Cliprdr`
- image clipboard formats
- file clipboard virtualization

### 4.2 Clipboard Set

`clipboard set --text <TEXT>` should send:

- `Message::Clipboard`
  - `format = Text`
  - `content = UTF-8 bytes`
  - `compress = false`
  - `width = 0`
  - `height = 0`

`Message::MultiClipboards` is optional for future multi-format support, but unnecessary for the initial text-only CLI.

### 4.3 Clipboard Get

There is no dedicated "clipboard request" message in the current message union for plain text clipboard retrieval. The protocol is event-driven.

Therefore, `clipboard get` should be defined as:

- return the latest text clipboard observed from the remote peer on the live session
- if no clipboard has yet been observed, wait for a short configurable window for an inbound clipboard event
- if still none arrives, return a cache-miss or timeout error

Daemon requirements:

- keep clipboard sync enabled through `OptionMessage`
- update `ClipboardCache` whenever inbound `Clipboard` or `MultiClipboards` arrives
- prefer the first `ClipboardFormat::Text` item when processing `MultiClipboards`

### 4.4 Clipboard Receive Semantics

Inbound clipboard handling rules:

- if `Message::Clipboard` with `format = Text`
  - decode as UTF-8 after decompression if needed
  - update cache
- if `Message::MultiClipboards`
  - scan for the first text clipboard
  - update cache from that item
- if `compress = true`
  - apply the RustDesk clipboard decompression path before UTF-8 decoding
- if format is not text
  - ignore for the text CLI, but keep logging for observability

### 4.5 Clipboard Permission Handling

If the remote side disables clipboard permission or does not emit clipboard events:

- `clipboard set` should return a permission or unsupported error if the protocol reports one
- `clipboard get` should return a timeout or unavailable error if no text clipboard arrives and cache is empty

## 5. Error Handling Design

### 5.1 Error Categories

Errors should be categorized by operation stage:

- connect failure
- transport closed
- auth failure
- terminal open failure
- terminal runtime failure
- clipboard unavailable
- timeout
- busy/conflict
- protocol decode error

Responses should preserve stage information so CLI output can say where the failure occurred.

### 5.2 Timeouts

Recommended timeout classes:

- connect timeout
  - rendezvous, relay, crypto, and login
- terminal open timeout
  - waiting for `TerminalResponse::Opened`
- shell inactivity timeout
  - daemon idle timeout for an attached shell with no client present
- exec completion timeout
  - waiting for sentinel
- clipboard wait timeout
  - waiting for first clipboard event when cache is empty
- terminal close timeout
  - waiting for `TerminalResponse::Closed`

Timeout behavior:

- connect timeout -> daemon startup fails, no live session
- shell inactivity timeout -> close terminal, keep transport if session remains healthy
- exec completion timeout -> close exec terminal and return partial output with timeout flag
- clipboard wait timeout -> no transport teardown; return an operation error only

### 5.3 Reconnection Policy

The daemon should not attempt transparent reconnection during an active shell or exec. Terminal state cannot be assumed recoverable.

Recommended policy:

- no automatic reconnect during `InteractiveShellActive`
- no automatic reconnect during `ExecActive`
- optional one-shot lazy reconnect for non-terminal commands only, using saved `ConnectionConfig`
- any reconnect attempt must clear stale terminal runtime state first

If reconnect is attempted:

1. mark runtime as reconnecting
2. rebuild transport using saved connection config
3. resend `OptionMessage`
4. restart inbound router
5. restore clipboard sync only
6. do not restore an active exec
7. do not silently restore an interactive shell unless future terminal persistence is intentionally implemented

### 5.4 Partial Reads

Terminal reads are arbitrary byte chunks. The daemon must not assume:

- one protobuf terminal data message equals one shell line
- one chunk is valid UTF-8
- prompts arrive in one piece
- sentinel appears in one chunk

Required handling:

- accumulate raw bytes
- search for sentinels across chunk boundaries
- decode to UTF-8 only for final user-facing rendering
- preserve raw bytes in memory until the command completes or fails

### 5.5 Unexpected Terminal Closure

If a `TerminalResponse::Closed` arrives:

- during shell:
  - forward closure to the attached CLI
  - return daemon runtime to `ConnectedIdle`
- during exec:
  - return partial output plus closure metadata
  - treat missing sentinel as abnormal completion

### 5.6 Protocol Errors And Unknown Messages

Inbound router behavior on unsupported messages:

- unknown message type -> log and ignore
- decode failure for one frame -> treat as connection corruption and fail the session
- terminal response for unknown terminal id -> log and ignore
- clipboard parse failure -> do not tear down transport; report clipboard-specific error

### 5.7 Busy And Concurrency Rules

Version 1 should enforce a simple concurrency model:

- one connected RustDesk text transport per daemon
- zero or one interactive shell at a time
- zero or one exec at a time
- `exec` and `shell` are mutually exclusive

Clipboard operations may run while no terminal is active. They should not be allowed to steal the encrypted stream reader from the inbound router.

## 6. Module Dependency Graph

### 6.1 Responsibilities

- `daemon.rs`
  - owns process lifecycle
  - owns UDS server
  - owns live runtime state
  - performs command dispatch
  - manages inbound router and shell attach sessions
- `session.rs`
  - defines CLI-to-daemon command contract
  - defines CLI-visible state and response schema
  - performs lightweight validation and status reporting
- `text_session.rs`
  - owns high-level text-mode remote session operations
  - coordinates connect, option setup, terminal open/close, exec orchestration
- `terminal.rs`
  - encodes terminal actions
  - decodes terminal-specific protobuf payloads
  - does not own the daemon-wide inbound read loop
- `connection.rs`
  - rendezvous, relay, crypto, login
- `crypto.rs`
  - encrypted stream wrapper
- `proto.rs`
  - generated RustDesk protobuf types
- `transport.rs`
  - TCP transport and framing

### 6.2 Dependency Graph

```text
CLI
  -> daemon.rs
      -> session.rs              (command contract, status)
      -> text_session.rs         (text-session orchestration)
          -> connection.rs       (rendezvous + relay + auth)
              -> rendezvous.rs
              -> crypto.rs
              -> proto.rs
              -> transport.rs
          -> terminal.rs         (terminal action/response helpers)
              -> crypto.rs
              -> proto.rs
              -> transport.rs
      -> proto.rs                (message decode in inbound router)
      -> crypto.rs               (encrypted stream ownership)
      -> transport.rs            (underlying socket transport)
```

### 6.3 Data Flow Graph

```text
connect
  CLI -> daemon control request
  daemon -> connection.rs
  connection.rs -> encrypted transport
  daemon -> text option setup
  daemon -> inbound router start

shell
  CLI -> daemon shell attach request
  daemon -> terminal open
  CLI stdin -> daemon -> TerminalAction::Data
  TerminalResponse::Data -> inbound router -> shell stream -> CLI stdout

exec
  CLI -> daemon exec request
  daemon -> terminal open
  daemon -> TerminalAction::Data(command + sentinel)
  TerminalResponse::Data -> inbound router -> exec collector
  daemon -> terminal close
  daemon -> SessionResponse

clipboard set
  CLI -> daemon clipboard-set request
  daemon -> Message::Clipboard

clipboard get
  remote -> Message::Clipboard or Message::MultiClipboards
  inbound router -> clipboard cache
  CLI -> daemon clipboard-get request
  daemon -> SessionResponse from cache or wait window
```

## 7. Recommended First Implementation Milestones

1. Make the daemon own a real text connection instead of stubbed connect state.
2. Introduce daemon runtime state with a live `TextSession`.
3. Add the single inbound router so terminal and clipboard messages can coexist safely.
4. Split terminal-open from connection establishment at the text-session layer.
5. Implement interactive shell UDS attach mode.
6. Implement exec using ephemeral terminal plus sentinel completion.
7. Add clipboard cache updates from inbound `Clipboard` and `MultiClipboards`.
8. Add lazy reconnect only if needed for non-terminal commands.

## 8. Final Design Summary

The daemon should become the sole owner of a real RustDesk text transport. `connect` must establish the encrypted session through `connection.rs`, enable text-mode options, and start a single inbound protobuf router. `shell` must open a dedicated terminal and upgrade the local UDS connection into a bidirectional byte stream. `exec` must open a separate ephemeral terminal, use a daemon-generated sentinel to detect command completion, and return deterministic output plus exit status. Clipboard support must use inbound/outbound `Message::Clipboard` and `Message::MultiClipboards`, with daemon-side caching because the protocol is push-based for plain text clipboard state. The result is a clean separation: `session.rs` remains the control plane, `daemon.rs` owns lifecycle and concurrency, `text_session.rs` owns high-level remote text behavior, and `terminal.rs` remains the terminal message codec.
