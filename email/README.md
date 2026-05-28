# email

Email worker for the iii engine. SMTP send + real-time IMAP read with `IDLE`
push. Every agent that needs to send a transactional email, scan an inbox, or
react the moment a new message lands goes through `email::*`, so account
binding, credential lookup, the SMTP transport, and the IMAP `IDLE` socket
lifecycle live in one place.

The worker refuses to fall back to polling. If an IMAP server does not
advertise the `IDLE` capability, the supervisor fails at startup with
`E610`. Inbound messages flow through the `email::new-mail` trigger type,
which any sibling worker can subscribe to and have a function fired within
milliseconds of a server-side `EXISTS` push.

Credentials are never persisted by this worker. Every IMAP login and every
`email::send` re-fetches the secret from
[`harness/auth-credentials`](../harness) under provider key
`email::<account>`. Pair with `harness/auth-credentials` for any real
deployment.

## Install

```bash
iii worker add harness/auth-credentials
iii worker add email
```

`iii worker add` fetches the binary, writes a config block into the
engine's `config.yaml`, and the engine starts the worker on the next
`iii worker start`.

For surfacing `email::*` to LLM agents, pair with
[`skills`](../skills):

```bash
iii worker add skills
```

## Quickstart

Send a message from an iii worker:

```rust
use iii_sdk::{register_worker, InitOptions, protocol::TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "email::send".into(),
            payload: json!({
                "account": "support",
                "to": ["recipient@example.com"],
                "subject": "Your ticket has been updated",
                "text": "Hi — thanks for reaching out."
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

```typescript
import { registerWorker } from 'iii-sdk'

const worker = registerWorker('ws://localhost:49134')

const result = await worker.trigger({
  function_id: 'email::send',
  payload: {
    account: 'support',
    to: ['recipient@example.com'],
    subject: 'Your ticket has been updated',
    text: 'Hi — thanks for reaching out.',
  },
})

console.log(result)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

result = worker.trigger({
    "function_id": "email::send",
    "payload": {
        "account": "support",
        "to": ["recipient@example.com"],
        "subject": "Your ticket has been updated",
        "text": "Hi — thanks for reaching out.",
    },
})

print(result)
```

React to inbound mail in real time — register a function and subscribe
it to `email::new-mail` in your worker's `iii.worker.yaml`:

```yaml
triggers:
  - type: email::new-mail
    function_id: support::ingest
    config:
      account: support
      folder: INBOX
```

Your handler receives one event per new message — fan-out is event-driven
off the IMAP server's `EXISTS` push, not a timer:

```typescript
iii.registerFunction('support::ingest', async (event) => {
  // event.account, event.folder, event.uid, event.from,
  // event.subject, event.snippet, event.ts
  // Your business logic here.
  return { ok: true }
})
```

Other entry points: `email::accounts::list`, `email::list`, `email::get`,
`email::search`, `email::flag`, `email::move`, `email::attachment::get`.

## Configuration

```yaml
accounts:
  # Send-only account: only the smtp: block, provider: smtp.
  support:
    provider: smtp
    from: "Support <support@example.com>"
    smtp:
      host: smtp.example.com
      port: 587
      starttls: true

  # Two-way account: both smtp: and imap: blocks, provider: imap.
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

# Hard caps applied before any network call.
limits:
  max_attachment_bytes: 26214400        # 25 MiB
  max_recipients: 100                   # to + cc + bcc combined
  send_timeout_ms: 30000                # SMTP RTT cap
  imap_connect_timeout_ms: 15000        # initial TCP+TLS+LOGIN budget
```

Credentials live in `harness/auth-credentials` under provider key
`email::<account>` with shape
`{ "type": "api_key", "username": "...", "password": "..." }`. The worker
calls `auth::get_token` on every connect; rotation is automatic from the
caller's perspective.

## Trying it locally

Two end-to-end paths. Path A spins a local IMAP+SMTP container so you can
exercise every function without touching a real provider. Path B points
the worker at a real IMAP server (any provider that supports `IDLE` and
app passwords).

### Path A — local GreenMail container

[GreenMail](https://greenmail-mail-test.github.io) is a single-container
mail server that speaks SMTP, IMAP, and POP3, advertises `IDLE`, and
ships a REST inspection API. Useful for SMTP-only validation; the IMAP
TLS port presents a self-signed cert and Java-side cipher set that
modern `rustls(ring)` will reject — for full IMAP coverage use Path B.

**1. Start GreenMail.**

```bash
docker run -d --name greenmail \
  -p 3025:3025 -p 3143:3143 -p 3993:3993 -p 8080:8080 \
  -e GREENMAIL_OPTS='-Dgreenmail.setup.test.all \
    -Dgreenmail.hostname=0.0.0.0 \
    -Dgreenmail.users=alice:alicepass@local.test,bob:bobpass@local.test \
    -Dgreenmail.auth.disabled=false -Dgreenmail.verbose' \
  greenmail/standalone:2.1.0
```

Verify it advertises `IDLE`:

```bash
(echo "a CAPABILITY"; sleep 1; echo "a LOGOUT") | nc localhost 3143
# expect: * CAPABILITY ... IDLE ...
```

**2. Write a send-only config** (`config.yaml`):

```yaml
accounts:
  alice:
    provider: smtp
    from: "Alice <alice@local.test>"
    smtp:
      host: localhost
      port: 3025
      starttls: false
  bob:
    provider: smtp
    from: "Bob <bob@local.test>"
    smtp:
      host: localhost
      port: 3025
      starttls: false

limits:
  max_attachment_bytes: 26214400
  max_recipients: 100
  send_timeout_ms: 30000
  imap_connect_timeout_ms: 15000
```

**3. Build the worker.**

```bash
cargo build --release
```

**4. Seed credentials into `harness/auth-credentials`.**

```bash
iii trigger auth::set_token \
  provider=email::alice \
  credential='{"type":"api_key","username":"alice","password":"alicepass"}'

iii trigger auth::set_token \
  provider=email::bob \
  credential='{"type":"api_key","username":"bob","password":"bobpass"}'
```

**5. Start the worker** (foreground; watch logs).

```bash
RUST_LOG=email=info,info ./target/release/email --config ./config.yaml
```

Expected boot log:

```
INFO email: loaded config from ./config.yaml accounts=2
INFO email: connecting to iii engine url=ws://localhost:49134
INFO email: email registered 8 functions and 1 trigger type;
            0 IMAP connections supervised
INFO iii_sdk::iii: iii connected address=ws://localhost:49134
```

**6. Exercise the functions** (separate shell).

```bash
# Discovery
iii trigger email::accounts::list

# Send alice → bob
iii trigger email::send \
  account=alice \
  'to=["bob@local.test"]' \
  subject='smoke 1' \
  text='hello from email worker'
# → { "message_id": "OK" }

# Verify GreenMail received it
curl -s http://localhost:8080/api/user/bob@local.test/messages | jq

# Error-code matrix — each should return the documented E-code envelope
iii trigger email::send account=alice 'to=[]' subject=x text=x
# → E601 "at least one recipient required in `to`"

iii trigger email::send account=alice 'to=["bob@local.test"]' subject=x
# → E604 "provide at least one of `html` or `text`"

iii trigger email::send account=ghost 'to=["x@x"]' subject=x text=x
# → E600 "unknown account `ghost`"
```

**7. Tear down.**

```bash
iii trigger auth::delete_token provider=email::alice
iii trigger auth::delete_token provider=email::bob
docker rm -f greenmail
```

### Path B — real IMAP server (the full IDLE path)

This is the path that actually validates real-time `IDLE` push. Works
against any provider that supports `IDLE` + app passwords: Gmail,
iCloud Mail, Fastmail, Yahoo, Outlook (with app password), or a
self-hosted Dovecot. The examples use Gmail; substitute hosts +
credentials for any other provider.

**Prerequisites.**

- Account credentials with 2-Step Verification enabled.
- An app password generated for the account (Gmail:
  https://myaccount.google.com/apppasswords ; takes 30s once 2SV is on).
- IMAP enabled on the account (Gmail: ships off by default —
  https://mail.google.com/mail/u/0/#settings/fwdandpop → "Enable IMAP").

**1. Write a Gmail-shaped config** (`config.yaml`):

```yaml
accounts:
  gmail:
    provider: imap
    from: "Your Name <you@example.com>"
    smtp:
      host: smtp.gmail.com
      port: 587
      starttls: true
    imap:
      host: imap.gmail.com
      port: 993
      tls: true
      folders: ["INBOX"]

limits:
  max_attachment_bytes: 26214400
  max_recipients: 100
  send_timeout_ms: 30000
  imap_connect_timeout_ms: 15000
```

**2. Seed your credential.** The app password lives in
`harness/auth-credentials`, never in this worker's process or files:

```bash
# Read the password from stdin to keep it out of shell history.
read -rs -p "App password: " APP_PW; echo
iii trigger auth::set_token \
  provider=email::gmail \
  credential="{\"type\":\"api_key\",\"username\":\"you@example.com\",\"password\":\"$APP_PW\"}"
unset APP_PW
```

**3. Start the worker.**

```bash
RUST_LOG=email=info,info ./target/release/email --config ./config.yaml
```

Expected boot log (the second line is the critical one):

```
INFO email: email registered 8 functions and 1 trigger type;
            1 IMAP connections supervised
INFO email::provider::imap::connection: imap session ready
            (IDLE supported) account=gmail folder=INBOX host=imap.gmail.com
```

If you see `E610 IMAP server lacks IDLE` → wrong provider. If you see
`E616 imap login failed` → wrong username or password (verify both with
`(echo 'a LOGIN <user> "<pw>"'; sleep 2; echo 'a LOGOUT') | openssl
s_client -quiet -crlf -connect imap.gmail.com:993`).

**4. Discover the configured account.**

```bash
iii trigger email::accounts::list
# → { "accounts": [ {
#       "name": "gmail",
#       "from": "Your Name <you@example.com>",
#       "can_send": true,
#       "can_read": true,
#       "folders": ["INBOX"]
#     } ] }
```

**5. Send a message to yourself** (so `IDLE` on `INBOX` will fire).

```bash
iii trigger email::send \
  account=gmail \
  'to=["you@example.com"]' \
  subject='iii email smoke test' \
  text='Safe to delete.'
# → { "message_id": "2.0.0 OK <queue-id> - gsmtp" }
```

**6. Watch the worker log for the IDLE push** (real-time test — should
fire within ~30s of delivery).

```
INFO email::provider::imap::idle: imap IDLE: server pushed data;
     new uids fetched account=gmail folder=INBOX new_uid_count=1
     high_water=<N>
INFO email::provider::imap::idle: imap IDLE: dispatching email::new-mail
     event account=gmail folder=INBOX uid=<N+1> from="you@example.com"
     subject="iii email smoke test"
```

Two log lines per inbound message. If you registered a subscriber on
`email::new-mail`, its function fires here.

**7. Exercise the read functions.** Note the `uid` from the IDLE log
line above:

```bash
# Page the newest 5
iii trigger email::list account=gmail folder=INBOX limit=5

# Read one message
iii trigger email::get account=gmail folder=INBOX uid=<N+1>
# → { "from": ..., "to": [...], "subject": ..., "html": ...,
#     "text": ..., "attachments": [...] }

# Mark seen
iii trigger email::flag account=gmail folder=INBOX uid=<N+1> flag=seen

# Unmark
iii trigger email::flag account=gmail folder=INBOX uid=<N+1> flag=seen add=false

# Move to another folder (folder must already exist on the server)
iii trigger email::move account=gmail folder=INBOX uid=<N+1> dst_folder=Archive
```

`email::search` and `email::attachment::get` use a streaming response
channel; they're invoked by sibling workers passing a
`StreamChannelRef`, not directly from `iii trigger`. See
[`skills/SKILL.md`](skills/SKILL.md) for the full agent-facing surface.

**8. Subscribe a function to `email::new-mail`** to see end-to-end
fan-out. From any other worker, add:

```yaml
triggers:
  - type: email::new-mail
    function_id: my-worker::on_mail
    config:
      account: gmail
      folder: INBOX
```

Then send yourself another message; your worker's `my-worker::on_mail`
fires within milliseconds of the IDLE push above.

**9. Clean up.**

```bash
iii trigger auth::delete_token provider=email::gmail
# Stop the worker (Ctrl+C). Revoke the app password in the provider's
# UI when the test session is done.
```

## Triggers

This worker registers one subscribable trigger type:

| Name | Fires when |
|---|---|
| `email::new-mail` | IMAP `IDLE` push delivers a new message to a configured `(account, folder)`. |

Subscriber config:

```yaml
triggers:
  - type: email::new-mail
    function_id: <your-function-id>
    config:
      account: <account-name-from-email-config>
      folder: INBOX                # optional, default "INBOX"
      handler_timeout_ms: 30000    # optional, default 30000
```

Payload your function receives per new message:

```json
{
  "account":    "gmail",
  "folder":     "INBOX",
  "uid":        2769,
  "message_id": "<abc@mx.google.com>",
  "from":       "sender@example.com",
  "subject":    "...",
  "snippet":    "first ~200 chars of body",
  "ts":         "2026-05-28T10:54:47Z"
}
```

The dispatch is event-driven: the `EXISTS` push wakes the worker,
which fans out via `iii.trigger(function_id, payload)` per subscriber.
No timer-based polling.

## Errors

Stable across the trigger boundary. Callers can `match` on the `code`
field of the structured error envelope.

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

