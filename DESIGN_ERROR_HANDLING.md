# DESIGN_ERROR_HANDLING

## Purpose

Define a unified error-handling architecture for `rustdesk-cli` that fixes the following QA bugs and prevents similar regressions:

- `BUG-013` errors go to stdout instead of stderr
- `BUG-009` exit code conflicts between Clap defaults and the product spec
- `BUG-012` `do` batch bypasses session checks
- `BUG-014` `disconnect` with no active session returns `0` instead of `2`

This document is design only. It specifies error routing, exit-code policy, batch validation, and formatting contracts for text and JSON output.

## Source Requirements

From [`SPEC.md`](/Users/qlss/Documents/Projects/rustdesk-cli/SPEC.md):

- all errors go to stderr
- exit codes:
  - `0` success
  - `1` connection error
  - `2` session error
  - `3` input error

From [`tests/qa/BUG_REPORT.md`](/Users/qlss/Documents/Projects/rustdesk-cli/tests/qa/BUG_REPORT.md):

- `BUG-013`: current error text is printed to stdout
- `BUG-009`: Clap uses exit `2`, conflicting with spec `2` for session errors
- `BUG-012`: `do` succeeds offline by bypassing session validation
- `BUG-014`: `disconnect` without a session returns success instead of a session error

## Design Goals

1. Every failure is classified once and mapped consistently.
2. All errors are emitted on stderr in both plain-text and JSON mode.
3. CLI parse/validation errors use spec exit code `3`, not Clap’s default `2`.
4. Batch execution obeys the same session rules as non-batch execution.
5. Daemon-originated errors remain structured all the way to the CLI.
6. Success payloads stay on stdout. Error payloads stay on stderr.

## Non-Goals

- redesigning the success output format
- changing the transport protocol between daemon and remote RustDesk peer
- fixing unrelated functional bugs such as stubbed `connect` or `exec`

## 1. Error Model

## 1.1 Error Type Hierarchy

Introduce one logical error hierarchy for the whole program.

Top-level categories:

- `ConnectionError`
  - rendezvous failure
  - relay failure
  - auth failure
  - daemon spawn failure
  - daemon socket unreachable when a command expected a live session transport
- `SessionError`
  - no active session
  - already connected
  - disconnect requested without active session
  - shell/exec conflict
  - command requires session but session is disconnected
- `InputError`
  - argument parsing failure
  - invalid command syntax
  - invalid user-provided values after parse
  - invalid batch structure
- `InternalError`
  - serialization failure
  - protocol invariant violation
  - unexpected daemon failure

Only the first three categories map to spec-defined exit codes. `InternalError` should be mapped conservatively:

- preferred mapping: `1` if it occurred in connection/session transport path
- otherwise `3` if it is effectively input-contract failure
- if an explicit internal-only code is ever desired later, it must be added to the spec first

For the current spec, do not invent a new public exit code.

## 1.2 Error Envelope

All layers should communicate errors in structured form, even if the final presentation is plain text.

Recommended logical error envelope:

- `kind`
  - `connection_error`
  - `session_error`
  - `input_error`
  - `internal_error`
- `message`
  - human-readable explanation
- `command`
  - top-level command name
- `step`
  - batch step index if applicable
- `details`
  - optional structured fields
- `source`
  - `cli`
  - `daemon`
  - `batch`

This envelope should exist conceptually across:

- CLI parse/validation
- daemon command handling
- batch planning/execution

## 1.3 Layer Responsibilities

CLI responsibilities:

- classify parse and local validation failures as `InputError`
- render all failures to stderr
- map error kind to public exit code

Daemon responsibilities:

- return structured command failures, not ambiguous strings
- classify no-session conditions as `SessionError`
- classify remote/network failures as `ConnectionError`

Batch responsibilities:

- preserve per-step error kind and step index
- stop immediately on first failing step

## 2. Stderr Routing Architecture

## 2.1 Core Rule

Success goes to stdout. Failure goes to stderr.

This rule applies to:

- plain-text mode
- JSON mode
- single-command mode
- batch mode

`BUG-013` exists because the current rendering path does not distinguish success and error streams. That must be fixed at the response rendering layer, not ad hoc at individual call sites.

## 2.2 Rendering Split

The renderer should choose output stream from response status, not from output mode.

Rendering contract:

- successful response:
  - plain text -> stdout
  - JSON -> stdout
- error response:
  - plain text -> stderr
  - JSON -> stderr

This keeps Unix semantics intact and still allows machine-readable error capture in JSON mode via stderr.

## 2.3 Daemon To CLI Error Propagation

Daemon errors should not be flattened into plain strings too early.

Current problem:

- daemon returns `SessionResponse`
- CLI converts that into a generic string and prints it through the normal output path

Desired flow:

1. daemon classifies failure into structured error kind
2. daemon serializes structured error in its UDS response
3. CLI deserializes structured error
4. CLI maps kind to exit code
5. CLI renders the error envelope to stderr

The daemon should never be responsible for deciding stdout vs stderr. That is a CLI rendering concern.

## 2.4 Daemon Spawn Errors

Errors that happen before the UDS command path exists, such as daemon startup failure during `connect`, are still CLI-visible errors and must follow the same rule:

- classify as `ConnectionError`
- print to stderr
- exit `1`

## 3. Exit Code Mapping Strategy

## 3.1 Public Exit Code Table

Public contract:

- `0` success
- `1` connection error
- `2` session error
- `3` input error

This mapping must be enforced centrally. Individual commands should not choose integer exit codes directly.

## 3.2 Clap Reconciliation Strategy

Clap’s default behavior conflicts with the spec because it uses exit code `2` for parse errors.

Recommended design:

- stop using automatic `Cli::parse()` as the final authority for exit behavior
- switch to a parse path that allows interception of Clap errors before exit
- translate Clap parse/validation failures into the program’s `InputError`
- emit those failures using the normal renderer
- exit with `3`

Special handling:

- `--help` and `--version` remain successful informational exits with code `0`
- only actual parse/usage failures map to `3`

This resolves `BUG-009` without changing the external spec.

## 3.3 Mapping By Error Kind

Recommended mapping function:

- `ConnectionError` -> `1`
- `SessionError` -> `2`
- `InputError` -> `3`
- `InternalError` -> derive conservatively from operation context, defaulting to `1` for runtime failures

The key rule is that "no active session" is always `SessionError`, never a generic connection failure.

## 3.4 Disconnect With No Session

`BUG-014` should be fixed by classifying offline `disconnect` as:

- kind: `session_error`
- message: `No active session`
- command: `disconnect`
- exit code: `2`

Rationale:

- the spec explicitly defines session errors as "no active session, disconnected"
- `disconnect` with no session is not success
- it is also not an input error
- it is not a network or auth problem, so it should not be `1`

The QA note says PM called this "connection error", but the spec-defined numeric meaning is still session/no-active-session and therefore should remain exit `2`.

## 4. Batch Error Handling Architecture

## 4.1 Current Failure Mode

`BUG-012` exists because the current `do` path:

- parses tokens into steps
- synthesizes success responses locally
- never consults daemon/session state

That means batch semantics are disconnected from command semantics.

## 4.2 Required Batch Pipeline

The `do` command needs three distinct phases:

1. parse
2. preflight validation
3. execution

### Parse Phase

Responsibilities:

- tokenize and build typed steps
- detect malformed batch syntax

Failures here are:

- `InputError`
- exit `3`
- rendered to stderr

### Preflight Validation Phase

Responsibilities:

- determine whether each step requires an active session
- determine whether the batch itself changes session state
- validate impossible sequences before partial execution where appropriate

Important rule:

- preflight must be semantic, not just syntactic

Examples:

- `do type hello key enter` with no active session
  - fail before step 1 executes
  - `SessionError`, exit `2`
- `do connect 123 type hello`
  - allowed, because step 1 creates session state for step 2
- `do disconnect type hello`
  - step 1 may succeed or fail based on real state; if it succeeds, step 2 must fail with `SessionError`

### Execution Phase

Responsibilities:

- execute steps in order using the same command path as normal non-batch commands
- stop on first failure
- emit batch error with failing step index

This preserves PM’s answer in the bug report:

- stop immediately
- return the error for that step
- skip remaining steps

## 4.3 Placement Of Session Validation

Session validation must not live only inside individual top-level command handlers if batch mode can bypass them.

Recommended placement:

- primary validation lives in the shared command execution layer used by both normal commands and batch steps
- batch preflight uses command metadata to predict whether a live session is required
- actual authoritative check still happens at step execution time against daemon/session state

This gives two protections:

- preflight catches obviously invalid offline batches early
- execution catches dynamic state changes mid-batch

## 4.4 Batch Command Metadata

Each command should declare:

- `requires_session`
- `establishes_session`
- `terminates_session`
- `allowed_offline`

Example classification:

- `connect`
  - `requires_session = false`
  - `establishes_session = true`
- `disconnect`
  - `requires_session = true`
  - `terminates_session = true`
- `status`
  - `requires_session = false`
- `type`, `key`, `click`, `move`, `drag`, `capture`, `shell`, `exec`, `clipboard`
  - `requires_session = true`

Batch preflight should simulate state transitions using that metadata.

## 4.5 Batch Error Envelope

A batch failure should include:

- `command = "do"`
- `step.index`
- `step.command`
- `error.kind`
- `error.message`

Plain-text example shape:

`step=2 command=type session_error: No active session`

JSON example shape:

```json
{
  "ok": false,
  "command": "do",
  "step": {
    "index": 2,
    "command": "type"
  },
  "error": {
    "kind": "session_error",
    "message": "No active session"
  }
}
```

This payload belongs on stderr because it is an error.

## 5. Disconnect No-Session Semantics

## 5.1 Behavioral Rule

`disconnect` without an active daemon/session must be treated as failure, not idempotent success.

Required behavior:

- no active session -> `SessionError`
- message: `No active session`
- exit code: `2`
- emit on stderr

## 5.2 Validation Placement

This check should happen before reporting a successful disconnect.

Recommended flow:

1. CLI asks whether daemon/session exists
2. if not, synthesize structured `SessionError`
3. do not print `disconnected`
4. exit `2`

If daemon exists but command delivery fails:

- classify based on failure type
- likely `ConnectionError` if socket/daemon transport is broken
- likely `SessionError` if daemon explicitly says there is no active session

## 6. Formatting Design

## 6.1 Plain-Text Errors

Plain-text errors should be concise, single-line by default, and routed to stderr.

Recommended text format:

`<kind>: <message>`

Examples:

- `session_error: No active session`
- `input_error: invalid region width`
- `connection_error: authentication failed`

Batch plain-text format:

`step=2 command=type session_error: No active session`

Optional secondary detail lines may be added later, but the first line must stay stable for scripts and tests.

## 6.2 JSON Errors

JSON mode should still send errors to stderr, not stdout.

Recommended JSON error shape:

```json
{
  "ok": false,
  "command": "type",
  "error": {
    "kind": "session_error",
    "message": "No active session"
  }
}
```

Batch JSON error shape:

```json
{
  "ok": false,
  "command": "do",
  "step": {
    "index": 2,
    "command": "type"
  },
  "error": {
    "kind": "session_error",
    "message": "No active session"
  }
}
```

Recommended field naming:

- use `kind`, not overloaded `code`, for semantic category
- if a finer machine code is needed later, add:
  - `kind`
  - `code`
  - `message`

For the current design, `kind` is sufficient and aligns better with exit code mapping.

## 6.3 Success Formatting Boundary

Success formatting remains unchanged:

- plain-text success on stdout
- JSON success on stdout

The only architectural change is that error objects must never pass through the stdout success renderer.

## 7. Centralized Error Renderer

## 7.1 Required Design Change

The CLI needs one centralized renderer for:

- success text/json -> stdout
- error text/json -> stderr

Current flaw:

- rendering is split by output mode but not by success/failure

Desired rule:

- the renderer must branch first on success/failure
- only then on plain-text vs JSON

This single change fixes `BUG-013` globally instead of patching individual commands.

## 7.2 Command Flow Contract

Every command path should return one logical result type:

- success payload
- structured error payload

Not:

- pre-rendered text plus ad hoc exit code

This lets:

- batch compose results
- daemon forward structured failures
- CLI map exit codes consistently
- tests assert one canonical behavior

## 8. Recommended Architecture Changes

## 8.1 CLI Layer

CLI should own:

- Clap parse interception
- mapping parse errors to `InputError`
- stdout/stderr selection
- final exit code selection

## 8.2 Daemon Layer

Daemon should own:

- structured classification of runtime command failures
- no-session detection
- command-specific session conflict errors

Daemon should not own:

- stdout/stderr routing
- public exit code integers

## 8.3 Shared Error Definitions

Introduce one shared error definition used by both CLI and daemon command layers.

Shared definitions should include:

- error kind enum
- error envelope
- exit-code mapper
- text renderer
- JSON renderer

This ensures the non-batch and batch paths cannot drift apart.

## 9. Recommended Fix Order

1. Centralize error classification and rendering.
2. Intercept Clap parse errors and remap them to exit `3`.
3. Change all error rendering to stderr in both text and JSON mode.
4. Fix `disconnect` no-session classification to `SessionError` exit `2`.
5. Refactor `do` to use parse -> preflight -> execute with shared session checks.
6. Add step-indexed batch error envelopes.

## 10. Final Contract Summary

The public contract after this design is:

- all failures go to stderr
- all successes go to stdout
- exit codes are:
  - `0` success
  - `1` connection error
  - `2` session error
  - `3` input error
- Clap usage failures are remapped to input error exit `3`
- `disconnect` with no session is a `SessionError` exit `2`
- `do` validates and executes through the same shared command/session path as normal commands
- batch stops on first error and reports the failing step
- JSON errors are emitted on stderr with a structured envelope

This fixes `BUG-013`, `BUG-009`, `BUG-012`, and `BUG-014` at the architectural level rather than by command-specific patching.
