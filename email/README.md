# email

Email worker for the iii engine. SMTP send + real-time IMAP read with
`IDLE` push. The worker refuses to fall back to polling: an IMAP server
without `IDLE` fails fast at startup with `E610`. Inbound messages flow
through the `email::new-mail` trigger type, fanned out the moment the
server pushes `EXISTS`.

Credentials live in `harness/auth-credentials` under provider key
`email::<account>`. Pair both workers in any real deployment.

## Install

```bash
iii worker add harness/auth-credentials
iii worker add email
```

## Skills

Install the `email` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill email
```

Browse or install every worker skill at once:

```bash
npx skills add iii-hq/workers --list
npx skills add iii-hq/workers --all
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, protocol::TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());
    let result = worker.trigger(TriggerRequest {
        function_id: "email::send".into(),
        payload: json!({
            "account": "support",
            "to": ["recipient@example.com"],
            "subject": "Your ticket has been updated",
            "text": "Hi — thanks for reaching out."
        }),
        action: None,
        timeout_ms: Some(30_000),
    }).await?;
    println!("{result:#?}");
    Ok(())
}
```

```typescript
import { registerWorker } from 'iii-sdk'

const worker = registerWorker('ws://localhost:49134')
await worker.trigger({
  function_id: 'email::send',
  payload: {
    account: 'support',
    to: ['recipient@example.com'],
    subject: 'Your ticket has been updated',
    text: 'Hi — thanks for reaching out.',
  },
})
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")
worker.trigger({
    "function_id": "email::send",
    "payload": {
        "account": "support",
        "to": ["recipient@example.com"],
        "subject": "Your ticket has been updated",
        "text": "Hi — thanks for reaching out.",
    },
})
```

Other entry points: `email::accounts::list`, `email::list`, `email::get`,
`email::search`, `email::flag`, `email::move`, `email::attachment::get`.

## Configuration

```yaml
accounts:
  # Send-only: only smtp:, provider: smtp.
  support:
    provider: smtp
    from: "Support <support@example.com>"
    smtp:
      host: smtp.example.com
      port: 587
      starttls: true

  # Two-way: smtp: + imap:, provider: imap.
  inbox:
    provider: imap
    from: "Inbox <inbox@example.com>"
    smtp:
      host: smtp.example.com
      port: 587
      starttls: true
    imap:
      host: imap.example.com
      port: 993
      tls: true
      folders: ["INBOX"]

limits:
  max_attachment_bytes: 26214400        # 25 MiB
  max_recipients: 100                   # to + cc + bcc combined
  send_timeout_ms: 30000
  imap_connect_timeout_ms: 15000
```

Credentials are fetched on every connect from `harness/auth-credentials`
under provider key `email::<account>` with shape
`{ "type": "api_key", "username": "...", "password": "..." }`. For Gmail,
generate an app password at https://myaccount.google.com/apppasswords —
the worker accepts both spaced (`abcd efgh ijkl mnop`) and joined
(`abcdefghijklmnop`) formats.

## Triggers

| Name | Fires when |
|---|---|
| `email::new-mail` | IMAP `IDLE` push delivers a new message to a configured `(account, folder)`. |

Subscriber config:

```yaml
triggers:
  - type: email::new-mail
    function_id: my-worker::on-mail
    config:
      account: support
      folder: INBOX                # optional, default "INBOX"
      handler_timeout_ms: 30000    # optional, default 30000
```

Payload your function receives per inbound message:

```json
{
  "account":    "support",
  "folder":     "INBOX",
  "uid":        12345,
  "message_id": "<abc@mx.example.com>",
  "from":       "sender@example.com",
  "subject":    "Ticket #42",
  "snippet":    "first ~200 chars of body",
  "ts":         "2026-05-28T10:14:00+00:00"
}
```

The dispatch is event-driven off the server's `EXISTS` push — within
milliseconds of a new message landing in the watched folder.

## Local development & testing

```bash
# In one terminal: start the engine
iii

# In another: build & run the worker
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

The worker registers 8 functions + 1 trigger type, then spawns one
persistent IMAP+IDLE supervisor per `(account, folder)` configured with
`provider: imap`. Reads (`list`/`get`/`search`/`flag`/`move`/`attachment::get`)
borrow an on-demand session from a separate pool so the half-duplex IMAP
socket is never shared with the supervisor.

Seed credentials before exercising `email::send` or any IMAP function:

```bash
iii trigger auth::set_token \
  provider=email::support \
  credential='{"type":"api_key","username":"you@example.com","password":"<app-password>"}'
```

`--manifest` prints the registry-publish JSON without touching the engine:

```bash
cargo run -- --manifest | jq .
```

### Tests

```bash
cargo test                       # unit tests (config, manifest, triggers)
cargo test --test bdd            # cucumber: --manifest subprocess contract
```

`tests/bdd.rs` self-skips `@engine` scenarios when no engine is reachable
on `III_ENGINE_WS_URL` (default `ws://127.0.0.1:49134`), so contributor
laptops without a running engine still pass `@pure` scenarios.

### Verification before publishing

The full preflight checklist for binary workers
([`docs/sops/binary-worker.md`](https://github.com/iii-hq/workers/blob/main/docs/sops/binary-worker.md)):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./target/debug/email --manifest | jq .
```

## Errors

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
| `E610` | IMAP server lacks IDLE — refusing to fall back to polling |
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
| `E626` | Attachment payload malformed (e.g. invalid base64) |
| `E627` | `email::move` partial: copy succeeded but `STORE \Deleted` failed — message in BOTH folders, reconcile |
| `E699` | Not yet implemented in 0.1.0 |
