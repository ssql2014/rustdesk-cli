# DESIGN_TEXT_OPTIMIZATIONS

## Purpose

Define the optimization layer that sits on top of the text-session architecture in [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md).

This document covers:

- which text-mode optimizations are feasible with the RustDesk protocol
- the recommended design approach for each
- protocol limits and required workarounds
- module structure for the optimization layer
- recommended implementation order

This is design only. No implementation details here are intended as code.

## Context

The base text-session architecture already establishes these invariants:

- the daemon owns the live RustDesk connection
- there is exactly one inbound router reading the encrypted stream
- interactive shell and exec are daemon-managed terminal operations
- clipboard is event-driven and cached daemon-side

The optimization layer must preserve those invariants. In particular:

- no optimization may introduce a second reader on the encrypted stream
- terminal optimizations must remain byte-oriented, not line-oriented
- local UX optimizations must degrade safely when network conditions are poor

## Protocol Capabilities Relevant To Optimization

The RustDesk protobuf surface gives us the following useful hooks:

- `TerminalAction::Open(OpenTerminal { terminal_id, rows, cols })`
- `TerminalAction::Data(TerminalData { terminal_id, data, compressed })`
- `TerminalAction::Resize(ResizeTerminal { terminal_id, rows, cols })`
- `TerminalAction::Close(CloseTerminal { terminal_id })`
- `TerminalResponse::{Opened, Data, Closed, Error}`
- `Message::Clipboard`
- `Message::MultiClipboards`

The protocol directly supports:

- multiple logical terminal channels via `terminal_id`
- terminal resize via `ResizeTerminal`
- per-packet terminal payload compression via `TerminalData.compressed`
- out-of-band clipboard traffic via message union members

The protocol does not directly support:

- explicit remote terminal "exec without PTY" mode
- prompt-aware command completion
- server-acknowledged type-ahead
- semantic terminal diffs or state deltas
- mosh-style prediction or state reconciliation

## Optimization Tiers

## P0: Must Have

### 1. Minimal Keystroke Latency

#### Feasibility

Fully feasible.

RustDesk terminal input is already a byte stream. There is no protocol-level requirement to transform input into key events. The lowest-latency path is direct `TerminalAction::Data` writes.

#### Recommended Approach

Use a fast-path input pipeline:

1. local terminal enters raw mode
2. local input bytes are captured exactly as read
3. bytes are placed onto a very short-lived outbound input queue
4. queue is flushed immediately unless a micro-batching window is active
5. bytes are sent as `TerminalAction::Data`

Latency rules:

- default to immediate flush for the first byte after idle
- permit micro-batching only for bursts that occur within a very small budget
- never wait on line boundaries or UTF-8 boundaries

The optimization target is not lower total bytes. It is lower end-to-end key-to-remote-PTY delay.

#### Protocol Limitations

- no per-keystroke acknowledgment
- no server hint for remote PTY processing delay
- no protocol signal that a byte has been rendered remotely

Therefore, the design should optimize for send-side delay only and avoid fake correctness guarantees.

#### Module Structure

- `text_opt/input.rs`
  - raw local input capture
  - flush policy
  - burst detection
- `text_opt/metrics.rs`
  - local latency timestamps
  - queue depth instrumentation

### 2. Smart Buffering

#### Feasibility

Fully feasible.

Smart buffering is purely a client-side policy over `TerminalData` writes and routed output delivery.

#### Recommended Approach

Split buffering into input-side and output-side policies.

Input-side policy:

- immediate flush on isolated keystrokes
- micro-batch for burst typing
- force flush on:
  - Enter
  - Ctrl-C
  - Esc
  - arrow keys and navigation sequences
  - paste bursts
  - resize events

Output-side policy:

- forward remote output to the CLI as soon as it arrives
- coalesce only when:
  - multiple `TerminalResponse::Data` packets arrive in one scheduler slice
  - local stdout backpressure exists

The output side should optimize syscall count, not interactivity. It should not add visible delay.

#### Protocol Limitations

- terminal payloads are chunked arbitrarily
- no semantic distinction between paste, shell output, and screen redraws

Therefore, buffering decisions must be based on local timing and byte patterns, not protocol metadata.

#### Module Structure

- `text_opt/buffer.rs`
  - input micro-batching strategy
  - output coalescing strategy
  - backpressure hooks

### 3. Terminal Resize (SIGWINCH)

#### Feasibility

Fully feasible.

RustDesk terminal protocol directly supports `ResizeTerminal { terminal_id, rows, cols }`.

#### Recommended Approach

When the local terminal size changes:

1. CLI receives local `SIGWINCH`
2. CLI computes current rows and cols
3. CLI sends a resize event over the shell UDS stream
4. daemon forwards that as `TerminalAction::Resize`
5. remote PTY updates window size

Resize behavior:

- debounce very short resize storms
- keep only the latest size in a burst
- never reorder resize behind buffered input for long periods

Interactive shell needs real-time resize. Exec mode does not, unless future exec usage supports full-screen TUIs.

#### Protocol Limitations

- size is row/column based only
- no pixel geometry
- no explicit remote acknowledgment beyond normal terminal behavior

#### Module Structure

- `text_opt/resize.rs`
  - local resize event normalization
  - debounce policy
  - terminal-id aware forwarding

### 4. Raw PTY Passthrough

#### Feasibility

Fully feasible and required.

The terminal protocol carries opaque bytes. That is exactly what raw-mode shell transport needs.

#### Recommended Approach

Interactive shell mode should be byte-transparent:

- local TTY enters raw mode
- all stdin bytes are forwarded unchanged
- all remote terminal bytes are emitted unchanged
- no newline normalization
- no UTF-8 validation in the forwarding path
- no ANSI parsing in the transport path

This preserves:

- escape sequences
- cursor addressing
- colors
- alternate screen buffers
- tmux/vim/full-screen TUIs

Any rendering-aware logic, if ever added, must sit outside the transport path and remain optional.

#### Protocol Limitations

None at the transport level. The limitation is local: the CLI must not accidentally switch back to cooked line mode while the shell session is active.

#### Module Structure

- `text_opt/raw_tty.rs`
  - local terminal raw-mode lifecycle
  - cleanup guarantees on detach/error
- `text_opt/shell_stream.rs`
  - byte-transparent bridge between local TTY and daemon shell channel

### 5. Compression For Terminal Output

#### Feasibility

Protocol-feasible, implementation-dependent.

`TerminalData` includes a `compressed` boolean for both action and response payloads. That means RustDesk explicitly expects compressed terminal payloads to be possible. The open question is the exact compression algorithm expected by the remote peer. The brief requests zstd specifically, but the protobuf schema only signals a boolean, not an algorithm identifier.

#### Recommended Approach

Design for negotiated compression policy with conservative fallback:

- default mode: uncompressed terminal traffic
- optional optimized mode: enable compressed terminal payloads only if interoperability is confirmed against the RustDesk host
- abstract compression behind a terminal codec layer

Recommended codec design:

- outbound:
  - if compression policy is enabled and payload exceeds threshold, compress and set `compressed = true`
  - otherwise send raw bytes with `compressed = false`
- inbound:
  - if `compressed = true`, decode through the configured terminal compression codec
  - if codec fails, treat as protocol error for that terminal stream

Zstd recommendation:

- architect the codec layer so zstd can be plugged in
- do not assume zstd on the wire until confirmed against live RustDesk behavior

#### Protocol Limitations

Major limitation:

- `compressed` is only a boolean
- there is no field identifying the codec
- the schema does not document zstd specifically

Workaround:

- design the compression subsystem with a single configured codec per connection
- keep it disabled by default until verified
- do not mix codecs on one connection

#### Module Structure

- `text_opt/codec.rs`
  - terminal payload encode/decode interface
  - raw passthrough codec
  - future zstd codec
- `text_opt/compress.rs`
  - thresholds
  - policy selection
  - per-direction enablement

## P1: Should Have

### 6. Clipboard Sync

#### Feasibility

Fully feasible.

The base protocol already exposes `Message::Clipboard` and `Message::MultiClipboards`.

#### Recommended Approach

Keep clipboard as a daemon-managed side channel:

- inbound router updates clipboard cache whenever text clipboard traffic arrives
- `clipboard set` sends `Message::Clipboard`
- `clipboard get` reads from cache or waits briefly for an inbound event

Optimization angle:

- treat clipboard as low-latency but non-blocking
- avoid coupling clipboard logic to active terminal channels
- avoid clipboard fetches that compete with shell traffic for the stream reader

Recommended clipboard policy:

- enable clipboard in `OptionMessage`
- cache the latest text clipboard
- optionally notify attached shell clients of remote clipboard changes in the future

#### Protocol Limitations

- plain text clipboard retrieval is push-based, not explicit request/response
- `MultiClipboards` may carry several formats

Workaround:

- standardize on `ClipboardFormat::Text`
- use daemon cache as the authoritative query source

#### Module Structure

- `text_opt/clipboard.rs`
  - inbound clipboard cache
  - outbound clipboard send helper
  - wait-window logic for `clipboard get`

### 7. Command Execution Mode

#### Feasibility

Feasible, but only as PTY-backed exec, not as a true "no-PTY" remote subprocess API.

RustDesk terminal protocol gives us PTY channels, not a separate remote exec RPC.

#### Recommended Approach

Use a specialized exec mode on top of terminal channels:

- open a fresh terminal
- optionally wait for initial shell readiness
- inject command bytes plus a daemon sentinel
- collect bytes until sentinel is observed
- parse exit status
- close terminal

Optimization angle:

- use exec-specific terminal settings
- disable local raw-mode shell bridge
- aggressively coalesce output for result packaging
- favor deterministic completion over minimal visible latency

Exec mode should be distinct from interactive shell mode in scheduling, buffering, and lifecycle.

#### Protocol Limitations

- no native "run command and return stdout" message
- no remote exit code field outside terminal semantics
- no guarantee about shell prompt or shell flavor

Workaround:

- sentinel-based completion
- ephemeral terminal per exec
- explicit timeout and abnormal-close handling

#### Module Structure

- `text_opt/exec.rs`
  - exec-specific terminal lifecycle
  - sentinel generation and detection
  - output packaging

### 8. Multiplexed `terminal_id` Channels

#### Feasibility

Protocol-feasible and strategically important.

The protocol carries `terminal_id` in open, resize, data, close, opened, closed, and error messages.

#### Recommended Approach

Adopt a daemon-side terminal multiplexer:

- one RustDesk transport
- many logical terminal channels keyed by `terminal_id`
- inbound router dispatches terminal responses to per-terminal mailboxes
- outbound writer tags writes with the correct terminal id

Recommended policy:

- support multiple exec terminals first
- allow at most one interactive shell terminal in the first iteration
- reserve additional channels for future background jobs or monitors

This gives:

- serial or concurrent exec jobs over one connection
- isolation between shell and exec traffic
- a clean path to per-channel buffering and policies

#### Protocol Limitations

- remote host behavior for many simultaneous terminals is not yet validated
- no explicit flow control per terminal channel

Workaround:

- cap concurrent terminal count locally
- implement per-terminal queues
- start with one shell plus a small exec concurrency limit

#### Module Structure

- `text_opt/mux.rs`
  - terminal-id allocator
  - per-terminal channel registry
  - route registration and teardown

## P2: Nice To Have

### 9. Type-Ahead

#### Feasibility

Partially feasible as a local UX optimization, not as a correctness-preserving protocol feature.

The RustDesk terminal protocol provides no prediction acknowledgment or rollback support.

#### Recommended Approach

Limit type-ahead to bounded local input buffering:

- if RTT rises above threshold, allow a small local queue of unsent bytes
- feed that queue into the normal input fast path as network capacity permits
- keep the queue byte-accurate and cancelable

Do not present type-ahead as confirmed remote state. It is only local staging.

Recommended constraints:

- small queue cap
- disabled for password prompts if detectable
- force flush on control sequences and Enter

#### Protocol Limitations

- no remote echo acknowledgment
- no conflict resolution if remote state diverges

Therefore, type-ahead must remain conservative and optional.

#### Module Structure

- `text_opt/predict.rs`
  - bounded input staging
  - latency-triggered enablement

### 10. Local Echo

#### Feasibility

Low-confidence and risky for general shell usage.

Local echo can improve feel when typing ordinary printable text into a typical shell prompt, but it is fundamentally unsafe for:

- password prompts
- shells with custom editing behavior
- full-screen TUIs
- remote applications that do not echo input

#### Recommended Approach

Do not enable general local echo in the first optimization layer.

If ever attempted:

- restrict to a narrow "predictive shell prompt" mode
- disable automatically on:
  - alternate screen usage
  - bracketed paste
  - password prompt heuristics
  - non-printable byte sequences

This is a future experiment, not a default behavior.

#### Protocol Limitations

- no server-side state model
- no echo/non-echo metadata
- no correction channel

Conclusion:

- feasible only as an optional heuristic
- not recommended for initial implementation

#### Module Structure

- fold into `text_opt/predict.rs` if ever built
- do not create a dedicated production module initially

### 11. Delta Updates

#### Feasibility

Not meaningfully feasible within the RustDesk terminal protocol as described.

RustDesk terminal transport is byte-stream oriented. It does not expose a structured terminal screen model or remote diff protocol. Mosh-like delta updates require:

- semantic terminal state tracking
- prediction
- state synchronization
- server cooperation

None of those exist in the current RustDesk terminal message set.

#### Recommended Approach

Do not attempt protocol-level delta updates in the first or second optimization phase.

If bandwidth reduction becomes necessary, prefer:

- terminal payload compression
- output coalescing
- better backpressure handling

Those are compatible with the existing protocol and significantly simpler.

#### Protocol Limitations

- no remote screen model
- no diff frames
- no sequence reconciliation
- no prediction rollback

Conclusion:

- not recommended
- outside the practical capability envelope of the current protocol

#### Module Structure

None for now.

If revisited later, it would be a research layer rather than a production optimization module.

## Recommended Implementation Order

### Phase A: P0 Core Interactivity

1. raw PTY passthrough
2. minimal keystroke latency fast path
3. SIGWINCH resize propagation
4. smart buffering
5. terminal compression abstraction with raw default

Rationale:

- this produces the largest immediate improvement for interactive shell usage
- it relies on protocol features that already exist
- it does not require speculative UX logic

### Phase B: P1 Session Efficiency

1. clipboard sync
2. exec mode specialization
3. multiplexed terminal channels

Rationale:

- these are high-value features for AI-agent workflows
- they depend on the daemon-owned router and terminal registry being stable

### Phase C: P2 Experimental UX

1. bounded type-ahead
2. guarded local echo experiments
3. explicitly skip delta updates unless protocol changes

Rationale:

- these optimizations are user-perception features, not core protocol wins
- the correctness risk is higher

## Recommended Module Structure

Optimization layer modules should sit above the transport and protobuf helpers and below CLI-specific presentation.

```text
daemon.rs
  -> text_session.rs
      -> text_opt/
          -> input.rs
          -> buffer.rs
          -> resize.rs
          -> raw_tty.rs
          -> shell_stream.rs
          -> codec.rs
          -> compress.rs
          -> clipboard.rs
          -> exec.rs
          -> mux.rs
          -> predict.rs
          -> metrics.rs
      -> terminal.rs
      -> connection.rs
      -> crypto.rs
      -> proto.rs
      -> transport.rs
```

Responsibility split:

- `text_session.rs`
  - orchestration entry points
  - daemon-facing high-level operations
- `text_opt/input.rs`
  - input fast path
- `text_opt/buffer.rs`
  - batching and flush policy
- `text_opt/resize.rs`
  - resize propagation
- `text_opt/raw_tty.rs`
  - local raw-mode lifecycle
- `text_opt/shell_stream.rs`
  - streaming bridge for interactive shell
- `text_opt/codec.rs`
  - terminal payload encode/decode contract
- `text_opt/compress.rs`
  - compression policy and thresholds
- `text_opt/clipboard.rs`
  - clipboard cache and synchronization
- `text_opt/exec.rs`
  - exec-mode orchestration
- `text_opt/mux.rs`
  - multiplexed terminal registry
- `text_opt/predict.rs`
  - experimental type-ahead and local echo logic
- `text_opt/metrics.rs`
  - observability for latency and batching

## Feasibility Summary

### Strongly Feasible Now

- minimal keystroke latency
- smart buffering
- SIGWINCH resize
- raw PTY passthrough
- clipboard sync
- PTY-backed exec mode
- multiplexed `terminal_id` channels

### Feasible With Validation Needed

- terminal payload compression
  - protocol supports a compressed flag
  - wire codec must be validated before enabling by default

### Experimental / High Risk

- type-ahead
- local echo

### Not Recommended Under Current Protocol

- delta updates in the mosh sense

## Final Recommendation

Build the optimization layer around three ideas:

1. keep the interactive shell byte-transparent and low-latency
2. centralize terminal routing, buffering, and multiplexing inside the daemon
3. treat predictive UX features as optional experiments, not core correctness features

The highest-value path is:

- raw passthrough
- low-latency input flushing
- resize propagation
- output buffering
- validated terminal compression
- clipboard cache
- specialized exec mode
- terminal multiplexing

Everything after that should be judged against one rule: if it can desynchronize local and remote terminal state, it stays optional and off by default.
