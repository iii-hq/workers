# email — architecture

Real-time email worker for the iii engine. SMTP send and IMAP read, with
`IDLE` push driving the `email::new-mail` trigger. Borrows the
router-shaped-process / persistent-connection pattern from `storage` and
the `ChannelWriter` streaming response shape from `mcp`.

## Boundaries

- **Owns**: SMTP transport, IMAP connections, `email::*` function
  registry, `email::new-mail` trigger type + dispatcher.
- **Does not own**: credentials (live in `harness/auth-credentials`),
  account-to-transport mapping for non-config-driven flows, OAuth
  redirect flow (future PRs), attachment storage (call out to `storage`
  if needed).
- **Refuses**: silently degrading IMAP `IDLE` to polling. If the
  server doesn't advertise `IDLE`, the supervisor task logs `E610` and
  loops the reconnect (which will keep failing) so the failure stays
  visible in observability instead of becoming a hidden poll loop.

## Process layout

```
                              ┌──────────────────────────────┐
                              │ iii engine (ws @ 49134)      │
                              └──────────────┬───────────────┘
                                             │
                                             │ register_worker
                                             ▼
┌──────────────────────────────── email worker (single process) ────────────────────────────────┐
│                                                                                                 │
│  main.rs:                                                                                       │
│    ├─ register_function(8) → "email::send", "::accounts::list", "::list", "::get",              │
│    │                          "::search", "::flag", "::move", "::attachment::get"               │
│    ├─ register_trigger_type("email::new-mail", Handler { registry })                            │
│    └─ for each (account, folder) in config:                                                     │
│         tokio::spawn(connection::run_until_shutdown) ◄── persistent IMAP supervisor             │
│                                                                                                 │
│  persistent supervisor (one per (account, folder)):                                             │
│    loop {                                                                                       │
│      open_and_select() → on E610 hold + log + retry                                             │
│      idle::run(session)                  // blocks on RFC 2177 IDLE wait                        │
│        ↓ (server pushes EXISTS)                                                                 │
│      fetch::uids_above(high_water)                                                              │
│      for uid in new_uids:                                                                       │
│        fetch::header_summary(uid)                                                               │
│        dispatcher.dispatch(Event { account, folder, uid, from, subject, snippet, ts })          │
│          → registry.subscribers_for(account, folder)                                            │
│          → iii.trigger(sub.function_id, payload)  for each                                      │
│      // idle re-arms                                                                            │
│    }                                                                                            │
│                                                                                                 │
│  on-demand pool (one per (account, folder)):                                                    │
│    list/get/search/flag/move/attachment::get borrow a Session                                   │
│    from ImapPool; supervisor's session is held separately so the                                │
│    half-duplex IMAP socket is never shared.                                                     │
│                                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Module map

| Module                                  | Purpose |
|-----------------------------------------|---------|
| `config`                                | YAML config: accounts + transports + limits |
| `manifest`                              | `--manifest` output (registry pipeline) |
| `handlers::*`                           | Public `email::*` function registry |
| `provider::smtp`                        | `lettre` transport: builder, attach, send |
| `provider::imap::connection`            | TLS connect, login, capability check (`E610` if no IDLE), supervisor loop |
| `provider::imap::idle`                  | RFC 2177 IDLE loop, EXISTS push → dispatcher |
| `provider::imap::fetch`                 | `UID FETCH` helpers: header summary, full body, streamed part bytes |
| `provider::imap::reconnect`             | Event-driven backoff between connect attempts (jittered exponential) |
| `provider::imap::mod` (`ImapPool`)      | `DashMap<(account, folder), Arc<Mutex<Option<Session>>>>` |
| `triggers::registry`                    | `(account, folder) → Vec<Subscriber>` indexed by instance id |
| `triggers::dispatcher`                  | `EngineDispatcher` — fan-out via `iii.trigger` per subscriber |
| `triggers::new_mail`                    | `TriggerHandler<email::new-mail>` registers/unregisters subscribers |

## Credentials

Email never persists secrets. Every connect (IMAP login, SMTP auth) calls:

```
iii.trigger("auth::get_token", { "provider": "email::<account>" })
```

The credential shape is:

```json
{ "type": "api_key", "username": "<user>", "password": "<pass>" }
```

OAuth-aware providers (Gmail, M365, JMAP) land in follow-up PRs and will
use the OAuth refresh-token path that `harness/auth-credentials`
already supports.

## Errors

`E600` family. Codes are stable across the trigger boundary so callers
can `match` on them.

| Code | When |
|------|------|
| `E600` | Unknown account name |
| `E601` | `email::send` with empty `to` |
| `E602` | Total recipients over `limits.max_recipients` |
| `E603` | Account missing the required transport block |
| `E604` | `email::send` with neither `html` nor `text` |
| `E605` | Attachment over `limits.max_attachment_bytes` |
| `E606` | `auth::get_token` upstream call failed |
| `E607` | No credential stored for the account |
| `E608` | Credential payload missing `username` / `password` |
| `E609` | Address parse / MIME build failure |
| `E610` | **IMAP server lacks IDLE — refusing to fall back to polling** |
| `E611` | Bad request payload (serde deserialization failure) |
| `E612` | IMAP `UID SEARCH` failed |
| `E613` | Folder not in account's `imap.folders` config |
| `E614` | IMAP connect / TLS handshake failed |
| `E615` | Plain (non-TLS) IMAP refused |
| `E616` | IMAP login failed |
| `E617` | IMAP `SELECT` failed |
| `E619` | IMAP body fetch / MIME parse failed |
| `E620` | SMTP send failed |
| `E621` | Response channel close failed |
| `E622` | Unknown flag name |
| `E623` | IMAP `STORE` failed |
| `E624` | IMAP `COPY` / `STORE \Deleted` fallback failed |
| `E625` | IMAP attachment-part fetch failed |
| `E699` | Not yet implemented in 0.1.0 (e.g. stream-source attachment send) |

## Roadmap

| PR | Adds | Status |
|---|---|---|
| 1 | SMTP send + IMAP read + IDLE push + new-mail trigger | this PR |
| 2 | Stream-source attachment send (closes E699 path) | next |
| 3 | Gmail OAuth + Pub/Sub push (push, not poll) | follow-up |
| 4 | Microsoft Graph + change-notification webhooks | follow-up |
| 5 | JMAP adapter (Fastmail) | follow-up |
| 6 | `email::draft::save/send`, `email::reply`, `email::forward` shortcuts | follow-up |
