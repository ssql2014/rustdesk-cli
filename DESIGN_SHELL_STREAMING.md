# DESIGN_SHELL_STREAMING

## Purpose

Define the architecture for interactive shell streaming between:

- local terminal
- CLI process
- daemon over Unix domain socket
- remote RustDesk terminal channel

This document builds on:

- [`DESIGN_TEXT_SESSION.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_SESSION.md)
- [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md)

It focuses specifically on the bidirectional streaming path for `rustdesk-cli shell`.

## Goals

1. Preserve raw terminal semantics end to end.
2. Support bidirectional streaming over a single UDS connection.
3. Propagate resize events correctly.
4. Minimize latency and buffering.
5. Ensure reliable cleanup on local exit, remote terminal close, signal delivery, or broken transport.
6. Integrate cleanly with the daemon-owned terminal multiplexer and optimization layer.

## Non-Goals

- redesigning `exec`
- shell multiplexing to multiple local clients at once
- GUI input/video integration
- application-level parsing of ANSI escape sequences

## 1. Local Terminal Raw Mode

## 1.1 Requirement

Interactive shell mode must place the local terminal into raw mode before bidirectional streaming begins.

Without raw mode:

- input is line-buffered
- Ctrl-C and Ctrl-D are intercepted locally instead of reaching the remote PTY when appropriate
- escape sequences are mangled or delayed
- full-screen applications such as `vim`, `tmux`, `less`, and shell line editors behave incorrectly

## 1.2 Feasible Local Terminal APIs

Two viable approaches:

- `libc tcgetattr/tcsetattr`
- `crossterm`

### Option A: `libc tcsetattr`

Advantages:

- minimal dependency surface
- direct control over termios flags
- predictable Unix behavior on Linux and macOS
- easier to reason about exact raw-mode semantics

Disadvantages:

- more manual cleanup logic
- more platform-specific branching

### Option B: `crossterm`

Advantages:

- safer and higher-level terminal mode management
- built-in terminal size helpers
- better ergonomics around event integration

Disadvantages:

- larger abstraction surface
- less direct control over exact low-level behavior
- still needs careful integration with raw byte streaming

## 1.3 Recommended Approach

Preferred design:

- use `libc tcsetattr` as the primary raw-mode mechanism
- optionally allow a thin portability wrapper later if terminal management grows more complex

Rationale:

- the shell streaming path is fundamentally Unix PTY-oriented
- exact raw-mode control matters more than high-level terminal abstractions
- the rest of the shell path already depends on low-level behavior such as signals and raw bytes

## 1.4 Raw Mode Lifecycle

The local CLI shell process should manage raw mode as a scoped lifecycle:

1. validate stdin/stdout are TTYs
2. capture original terminal settings
3. enable raw mode
4. run interactive shell session
5. restore terminal settings on every exit path

Raw mode must be restored on:

- normal shell exit
- remote terminal close
- UDS disconnect
- CLI process signal
- panic or early-return path where possible

## 1.5 Standard Streams Policy

Interactive shell mode should assume:

- stdin is a TTY
- stdout is a TTY
- stderr remains a normal diagnostic stream

If stdin or stdout is not a TTY:

- reject shell mode with an input/session error
- do not enter raw mode

This keeps shell semantics predictable and avoids half-interactive states.

## 2. Bidirectional UDS Protocol Framing

## 2.1 Why The Existing Protocol Is Insufficient

The existing daemon UDS protocol is:

- one JSON command line in
- one JSON response line out

That is insufficient for shell mode because shell mode needs:

- raw binary stdin bytes
- raw binary stdout bytes
- resize events
- close/error notifications
- long-lived full-duplex transport

Newline-delimited JSON cannot safely carry raw terminal byte streams without unnecessary escaping, framing ambiguity, and latency overhead.

## 2.2 Shell Session Upgrade Model

Recommended handshake:

1. CLI opens UDS connection to daemon.
2. CLI sends a JSON control request for `shell`.
3. Daemon validates session state and opens remote terminal.
4. Daemon sends a JSON ack line containing shell session metadata.
5. Both sides upgrade the same UDS socket into framed binary shell mode.

The initial JSON ack exists to preserve a clean control-plane boundary before switching to streaming.

## 2.3 Framed Message Types

Once upgraded, the UDS stream should carry framed messages with explicit type tags.

Required logical frame types:

- `stdin_data`
  - raw bytes from local CLI to daemon
- `stdout_data`
  - raw bytes from daemon to local CLI
- `resize`
  - rows and cols
- `close`
  - graceful end-of-session signal
- `error`
  - structured failure
- `heartbeat` optional
  - future keepalive/debugging use

The framing format must be binary-safe and length-prefixed.

## 2.4 Recommended Frame Format

Recommended abstract frame shape:

- `version`
- `type`
- `flags`
- `payload_length`
- `payload`

Requirements:

- payload may contain arbitrary bytes
- frames must be parseable without scanning for delimiters
- future frame types can be added without redesigning the stream

This is a design requirement, not a mandate for a specific on-wire struct.

## 2.5 Full-Duplex Behavior

The same UDS socket should remain concurrently usable in both directions:

- CLI write task sends `stdin_data` and `resize`
- CLI read task receives `stdout_data`, `close`, and `error`
- daemon write task sends `stdout_data`, `close`, and `error`
- daemon read task receives `stdin_data`, `resize`, and `close`

There should be no request/response turn-taking once shell streaming starts.

## 2.6 Relationship To Daemon Terminal Mux

The UDS shell stream maps to exactly one daemon-managed `terminal_id`.

Rules:

- one local shell attach per interactive terminal
- daemon associates the upgraded UDS connection with one interactive terminal session
- inbound RustDesk `TerminalResponse::Data` for that `terminal_id` is forwarded as UDS `stdout_data`
- UDS `stdin_data` is forwarded as RustDesk `TerminalAction::Data`

## 3. Signal Handling

## 3.1 SIGWINCH

`SIGWINCH` must be handled by the CLI shell process, not by the daemon.

Flow:

1. local terminal window changes size
2. CLI receives `SIGWINCH`
3. CLI reads current rows/cols from the local TTY
4. CLI sends a UDS `resize` frame
5. daemon converts that to `ResizeTerminal { terminal_id, rows, cols }`
6. remote PTY receives resize

This gives correct behavior for:

- `tmux`
- `vim`
- shell line editors
- any curses-style TUI

### Resize Coalescing

Resize storms should be coalesced:

- keep only the latest size if several resizes arrive close together
- do not block resize behind long stdout or stdin buffers

## 3.2 SIGINT And SIGTERM

Signals that terminate the local CLI process must trigger graceful cleanup.

Recommended handling:

- `SIGINT`
- `SIGTERM`

Default behavior in shell mode:

- first, attempt graceful shutdown of the local shell streaming session
- then restore local terminal mode
- then exit CLI

Important distinction:

- raw-mode Ctrl-C typed by the user should normally be forwarded to the remote PTY as bytes
- external process-level `SIGINT` delivered to the CLI should trigger local cleanup

Therefore:

- terminal byte input and OS signals must be treated as separate channels

## 3.3 Signal Handling Model

Recommended design:

- dedicated signal-listener task in the CLI
- signal events are forwarded into a local shutdown coordinator
- shutdown coordinator initiates:
  - local writer stop
  - UDS close or shell `close` frame
  - raw terminal restoration

Do not rely on default signal behavior once raw-mode shell has started.

## 4. Latency Path Analysis

## 4.1 Forward Path: Local Keystroke To Remote PTY

Complete path:

1. user presses key
2. local terminal driver produces raw byte(s)
3. CLI stdin reader receives bytes
4. CLI input buffer policy decides flush timing
5. CLI writes `stdin_data` frame to UDS
6. kernel copies bytes into UDS socket buffer
7. daemon UDS reader parses frame
8. daemon shell session writes `TerminalAction::Data`
9. daemon outbound transport queue serializes write
10. encrypted RustDesk stream sends framed payload
11. network transport carries bytes to remote host
12. RustDesk host injects bytes into remote PTY

Potential latency/buffering points:

- terminal driver read granularity
- CLI micro-batching window
- UDS socket buffering
- daemon scheduling delay
- daemon outbound write serialization
- encrypted transport framing
- network RTT and queueing
- remote PTY scheduling

## 4.2 Return Path: Remote PTY To Local Screen

Complete path:

1. remote PTY emits stdout/stderr bytes
2. RustDesk host packages them as `TerminalResponse::Data`
3. network transport sends to daemon
4. daemon inbound router reads encrypted payload
5. router dispatches by `terminal_id`
6. shell forwarder wraps bytes in UDS `stdout_data`
7. UDS kernel buffers deliver bytes to CLI
8. CLI shell reader parses `stdout_data`
9. CLI output coalescer decides immediate write or micro-coalesce
10. CLI writes bytes to local stdout TTY
11. local terminal renders bytes

Potential latency/buffering points:

- remote PTY flush behavior
- host-side RustDesk terminal packetization
- network RTT and queueing
- daemon inbound router scheduling
- UDS buffering
- CLI output coalescing
- local TTY write batching

## 4.3 Latency Principles

Optimization priorities from [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md):

- immediate send for isolated keystrokes
- micro-batch only for burst typing
- output coalescing only when it reduces syscall churn without adding visible lag
- resize must bypass long queues
- raw byte passthrough must avoid UTF-8 or line processing

## 4.4 Where Buffering Is Allowed

Buffering is acceptable at:

- CLI input micro-batch queue
- daemon outbound terminal write queue
- CLI output coalescer

Buffering must remain minimal and bounded.

Buffering is not acceptable as:

- line buffering
- prompt buffering
- waiting for UTF-8 completion before forwarding
- accumulating large full-screen frames before local output

## 5. Session End And Cleanup

## 5.1 Normal End Conditions

Shell mode may end normally via:

- user exits shell on remote side
- remote process closes PTY and daemon receives `TerminalClosed`
- user sends EOF such as Ctrl-D and remote shell exits
- local CLI requests shell close
- daemon disconnect command closes active terminal

## 5.2 Canonical Shutdown Sequence

Recommended shutdown sequence:

1. stop accepting new local stdin bytes
2. flush any tiny pending input buffer if appropriate
3. send shell `close` frame to daemon or close the UDS stream cleanly
4. daemon sends `CloseTerminal` to remote if terminal is still open
5. daemon waits briefly for `TerminalClosed`
6. daemon sends UDS `close` frame if still connected
7. CLI stops reader/writer tasks
8. CLI restores terminal mode
9. CLI exits with success unless an error caused shutdown

## 5.3 Remote TerminalClosed

If daemon receives `TerminalResponse::Closed` from the remote side:

- forward a UDS `close` frame to CLI
- include exit metadata if available
- transition daemon terminal runtime back to `ConnectedIdle`

CLI behavior:

- stop reading stdin for shell forwarding
- restore local terminal
- exit shell command cleanly

## 5.4 Ctrl-D

Ctrl-D is not a local shell-exit control. In raw mode it is just a byte sent to the remote PTY.

Therefore:

- local CLI should forward Ctrl-D unchanged
- if the remote shell interprets it as EOF and exits, the remote side will trigger the close path

This preserves correct Unix shell semantics.

## 5.5 Disconnect While Shell Is Active

If another CLI invocation triggers `disconnect` while interactive shell is active:

- daemon must treat shell terminal closure as part of disconnect
- daemon should close the interactive terminal first
- daemon should send `close` or `error` frame to the attached shell client if possible
- daemon should then tear down the session transport

The shell client should interpret this as remote session shutdown, restore the local terminal, and exit.

## 6. Error Recovery

## 6.1 UDS Breaks Mid-Session

If the UDS connection breaks while shell mode is active:

CLI side behavior:

- stop shell read/write tasks immediately
- restore local terminal mode
- report shell transport failure on stderr

Daemon side behavior:

- mark the interactive client as detached
- close the corresponding remote terminal unless future detach/re-attach semantics are explicitly supported
- return daemon terminal runtime to `ConnectedIdle` after remote close or timeout

Recommended policy for v1:

- UDS break ends the shell session
- do not leave orphaned interactive terminals running remotely

## 6.2 Remote PTY Dies

If the remote PTY dies or returns `TerminalError`:

- daemon forwards UDS `error` or `close`
- daemon clears active terminal state
- CLI restores local terminal and exits with session/runtime error

If partial stdout bytes arrived before failure:

- they should still be delivered to the CLI before final close if already received

## 6.3 Daemon Loses RustDesk Transport

If daemon loses the underlying RustDesk connection while shell is active:

- daemon sends UDS `error` frame if possible
- daemon closes the shell session
- CLI restores local terminal
- shell command exits with connection/session error

No automatic reconnect should occur during an active interactive shell.

## 6.4 Local Output Failure

If CLI cannot write remote terminal output to the local TTY:

- treat it as fatal to the shell command
- restore local terminal
- close UDS shell session

The shell path assumes stdout is a functioning terminal sink.

## 7. Integration With Text Optimizations

## 7.1 Raw Passthrough

From [`DESIGN_TEXT_OPTIMIZATIONS.md`](/Users/qlss/Documents/Projects/rustdesk-cli/DESIGN_TEXT_OPTIMIZATIONS.md):

- shell streaming must be byte-transparent
- no ANSI parsing in the transport path
- no line discipline in user space

This design adopts that requirement directly.

## 7.2 Input Buffering

The shell streaming stack should integrate with the optimization-layer input fast path:

- stdin reader produces raw bytes
- `text_opt/input.rs` decides immediate flush vs micro-batch
- framed UDS writer sends `stdin_data`

Special bytes that should force immediate flush:

- Enter
- Ctrl-C
- Esc
- arrow/function key sequences
- paste bursts larger than threshold

## 7.3 Output Coalescing

Shell streaming should integrate with output coalescing only at the CLI egress point:

- daemon forwards remote `TerminalResponse::Data` promptly
- CLI may merge adjacent `stdout_data` frames briefly to reduce syscall overhead
- coalescing must remain bounded and latency-sensitive

## 7.4 Resize Priority

Resize events should bypass or preempt normal buffered input where needed.

Rationale:

- full-screen TUIs behave poorly if resize is delayed behind queued typing

## 7.5 Compression

If terminal compression is enabled in the optimization layer:

- daemon handles compression/decompression at the RustDesk terminal boundary
- UDS shell framing between CLI and daemon should remain uncompressed by default

Rationale:

- UDS is local and low-overhead
- compression is most valuable across the network hop, not the local IPC hop

## 8. Recommended Module Structure

Recommended shell-stream-specific modules layered on top of the text optimization architecture:

```text
CLI shell command
  -> shell_client/
      -> raw_mode
      -> stdin_reader
      -> uds_frame_reader
      -> uds_frame_writer
      -> signal_handler
      -> resize_watcher
      -> stdout_sink

daemon.rs
  -> shell_server/
      -> shell_attach
      -> uds_shell_reader
      -> uds_shell_writer
      -> terminal_bridge
      -> terminal_close_manager

shared
  -> shell_frame
  -> shell_error
```

Responsibilities:

- CLI shell client
  - local terminal mode
  - local signal handling
  - local UDS streaming
- daemon shell server
  - attach validation
  - mapping UDS frames to terminal actions
  - mapping terminal responses to UDS frames
- shared shell frame definitions
  - binary-safe framing contract
  - frame type tags

## 9. Recommended End-To-End State Machine

CLI shell client states:

- `Init`
- `RawModePending`
- `AttachPending`
- `Streaming`
- `Closing`
- `RestoringTerminal`
- `Done`

Daemon interactive shell states:

- `Idle`
- `OpeningTerminal`
- `Attached`
- `ClosingTerminal`
- `Closed`

Key transitions:

- CLI `AttachPending` -> `Streaming` only after daemon ack
- daemon `OpeningTerminal` -> `Attached` only after `TerminalOpened`
- any transport failure -> CLI `RestoringTerminal`, daemon `ClosingTerminal`

## 10. Final Recommendation

Interactive shell mode should use:

- local raw terminal mode via low-level termios control
- a single long-lived upgraded UDS connection
- binary-safe framed full-duplex messages
- CLI-owned `SIGWINCH` handling that becomes `ResizeTerminal`
- daemon-owned terminal bridging keyed by one `terminal_id`
- strict cleanup on every exit path

The most important architectural constraint is this: raw local bytes, framed local IPC, and raw remote PTY bytes must remain separate layers. The CLI should manage the local terminal, the daemon should manage the remote terminal, and the UDS framing layer should only transport bytes and control events between them with minimal buffering and no semantic transformation.
