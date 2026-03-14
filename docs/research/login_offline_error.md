# Research Findings: RustDesk `login rejected: Offline`

**Date:** 2026-03-14
**Investigated by:** LEO (Design Engineer, Team Evas)
**Case:** Peer `308235080`
**Local servers:** `hbbs=115.238.185.55:50076`, `hbbr=115.238.185.55:50077`
**Upstream reference repo:** `https://github.com/rustdesk/rustdesk` cloned to `/tmp/rustdesk-leo`

## Conclusion

The peer-side `Offline` login rejection is primarily caused by a wrong `LoginRequest.username`, not by password failure and not by most target-machine settings.

For this case, `rustdesk-cli` is constructing the login packet incorrectly:

- it sends `username = ""`
- it sends `my_id = <target peer id>` instead of the caller's own ID

The upstream RustDesk peer checks `lr.username` very early in login handling. If `username` is neither:

- a valid IP string
- a valid `domain:port` string
- nor equal to the target machine's own RustDesk ID

then the peer responds with `LoginResponse.error = "Offline"`.

That means the current `Offline` error is best explained by our malformed `LoginRequest`, specifically the empty `username`.

## 1. What `rustdesk-cli` currently sends

In [`src/connection.rs`](../../src/connection.rs), `handshake_and_auth()` builds the encrypted `LoginRequest` at lines 336-356:

```rust
LoginRequest {
    username: String::new(),
    password: pw_hash.to_vec(),
    my_id: my_id.to_string(),
    my_name: "rustdesk-cli".to_string(),
    option: None,
    video_ack_required: false,
    session_id: rand_session_id(),
    version: env!("CARGO_PKG_VERSION").to_string(),
    os_login: None,
    my_platform: std::env::consts::OS.to_string(),
    hwid: Vec::new(),
    avatar: String::new(),
    union: None,
}
```

Relevant local references:

- `src/connection.rs:336-356`: `LoginRequest` construction
- `src/connection.rs:159`: `handshake_and_auth(..., &config.peer_id)` passes the target peer ID as `my_id`

So for peer `308235080`, `rustdesk-cli` is currently sending:

- `username = ""`
- `my_id = "308235080"`

This does not match the official client.

## 2. What the official RustDesk client sends

In `/tmp/rustdesk-leo/src/client.rs:2612-2702`, the official client builds `LoginRequest` differently:

- `username = pure_id`
- `my_id = Config::get_id()` or `id@server` for other-server mode

The key lines are:

```rust
let (my_id, pure_id) = if let Some((id, _, _)) = self.other_server.as_ref() {
    let server = Config::get_rendezvous_server();
    (format!("{my_id}@{server}"), id.clone())
} else {
    (my_id, self.id.clone())
};

let mut lr = LoginRequest {
    username: pure_id,
    password: password.into(),
    my_id,
    ...
};
```

For a normal connection to peer `308235080`, the official client intent is:

- `username = "308235080"` or the actual destination ID
- `my_id = <the caller's own RustDesk ID>`

## 3. Where the peer returns `Offline`

The peer-side rejection is in `/tmp/rustdesk-leo/src/server/connection.rs`.

The login handler receives `LoginRequest` at lines 2165+ and later checks `lr.username`.

At `/tmp/rustdesk-leo/src/server/connection.rs:2293-2299`:

```rust
if !hbb_common::is_ip_str(&lr.username)
    && !hbb_common::is_domain_port_str(&lr.username)
    && lr.username != Config::get_id()
{
    self.send_login_error(crate::client::LOGIN_MSG_OFFLINE)
        .await;
    return false;
}
```

This is the relevant behavior:

- the peer has already accepted the transport and received `LoginRequest`
- it checks `lr.username` before password validation
- if `username` does not identify the target correctly, it sends `Offline`

Search of the upstream source found one peer-side `send_login_error("Offline")` path in `src/server/connection.rs`, which makes this branch the authoritative cause of the observed login error.

## 4. What target-side settings can cause `Offline`

Peer-side `Offline` is not a general "machine is offline" signal at this stage of the flow. In the login handler, it is specifically tied to identity/routing mismatch.

Conditions that can trigger it:

1. `LoginRequest.username` is empty.
2. `LoginRequest.username` contains the wrong RustDesk ID.
3. The client uses a stale peer ID, and the target machine's current `Config::get_id()` is different.
4. The client is expected to connect by IP or `domain:port`, but `username` contains neither.

What does **not** trigger this specific `Offline` branch:

1. Wrong connection password.
2. Missing `option`.
3. Missing `os_login`.
4. Incorrect `my_id`.
5. File-transfer / terminal permission issues.

Those produce different errors later in the login flow, such as `Wrong Password`, `No Password Access`, or permission-specific failures.

## 5. Answer to the focus question

### Is this target-machine config or wrong `LoginRequest` fields?

Primary cause in this case: wrong `LoginRequest` fields.

Specifically:

- `username` is empty in `rustdesk-cli`, but the peer expects the destination ID (`308235080`) or an IP / `domain:port`.
- this alone is sufficient to produce `login rejected: Offline`

There is one target-side dependency:

- the peer compares against its own `Config::get_id()`

So a changed or stale target ID could also cause `Offline`, but given the current `rustdesk-cli` packet shape, the client bug is already enough to explain the failure.

## 6. Secondary bug found

`rustdesk-cli` also appears to send the wrong `my_id`.

- local code passes `&config.peer_id` into `handshake_and_auth()`
- that value is copied into `LoginRequest.my_id`
- the official client uses the caller's own ID there, not the destination ID

This is probably a real protocol bug, but it is not the branch that explains the current `Offline` rejection.

## 7. Practical implication for peer `308235080`

To avoid `Offline`, `rustdesk-cli` should send a login packet shaped like:

- `username = "308235080"`
- `my_id = <our own registered/local RustDesk ID>`

The password `Evas@2026` is not the deciding factor for the current error, because the peer returns `Offline` before password validation.
