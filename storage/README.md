# storage

Object storage for the iii engine over S3, GCS, R2, and a native local
filesystem backend. Streamed uploads, signed downloads, and `object-created` /
`object-deleted` triggers — all behind one `bucket:` name regardless of
the cloud underneath.

## Install

```bash
iii trigger compose::add worker=storage
```

`iii trigger compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.
The worker depends on `configuration` (where its settings live) and `secrets`
(which resolves `secret://` / `env://` credentials, see
[Credentials](#credentials-and-the-secrets-worker)).

## Quickstart

Use inline RPCs only for genuinely small values. Normal files should go
directly between the client and object storage through a signed upload or
download endpoint.

```ts
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')

await iii.trigger({
  function_id: 'storage::putObject',
  payload: {
    bucket: 'uploads',
    key: 'u/1/profile.jpg',
    body_base64: tinyValueBase64,    // small values only; 10 MiB hard limit
    content_type: 'image/jpeg',
  },
})

const { url, expires_at } = await iii.trigger({
  function_id: 'storage::presignUrl',
  payload: {
    bucket: 'uploads',
    key: 'u/1/next.jpg',
    method: 'PUT',
    expires_in_seconds: 600,
    content_type: 'image/jpeg',      // pinned into the signature
  },
})

await fetch(url, {
  method: 'PUT',
  headers: { 'Content-Type': 'image/jpeg' },
  body: file,
})

const { body_base64, content_type } = await iii.trigger({
  function_id: 'storage::getObject',
  payload: {
    bucket: 'uploads',
    key: 'u/1/profile.jpg',
  },
})

await iii.trigger({
  function_id: 'storage::deleteObject',
  payload: {
    bucket: 'uploads',
    key: 'u/1/profile.jpg',
  },
})                                  // idempotent: returns { deleted: false } if absent

const { content_type, etag, size, last_modified } = await iii.trigger({
  function_id: 'storage::headObject',
  payload: {
    bucket: 'uploads',
    key: 'u/1/profile.jpg',
  },
})                                  // fetches metadata only — no body download
```

For a multipart upload to a native local bucket, use `presignPost`:

```ts
const { url, fields } = await iii.trigger({
  function_id: 'storage::presignPost',
  payload: {
    bucket: 'scratch',
    key: 'videos/demo.mp4',
    content_type: 'video/mp4',
    expires_in_seconds: 600,
  },
})

const form = new FormData()
for (const [name, value] of Object.entries(fields)) form.append(name, value)
form.append('file', file)
await fetch(url, { method: 'POST', body: form })
```

> `putObject` and `getObject` serialize bytes as base64 inside an engine
> message. They buffer the complete value and base64 adds roughly 33% overhead.
> Both enforce a 10 MiB decoded hard limit per call, but they are intended for
> truly small values, not general file transfer. Use `presignPost`/a signed PUT
> for uploads and a signed GET from `presignUrl` for downloads. A client that
> cannot reach the provider (a phone outside the LAN of a local bucket, a GCS
> bucket without signing credentials) can read any object through `getObject`
> in ranges: `{ offset, length }`, see [Ranged reads](#storagegetobject--ranged-reads).

From a Rust worker:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

let iii = register_worker("ws://localhost:49134", InitOptions::default());

iii.trigger(TriggerRequest {
    function_id: "storage::putObject".into(),
    payload: json!({
        "bucket": "uploads",
        "key": "u/1/profile.jpg",
        "body_base64": file_b64,
        "content_type": "image/jpeg",
    }),
    action: None,
    timeout_ms: Some(5_000),
}).await?;
```

## Configuration

The storage worker gets its live configuration from the `configuration` worker,
not from a local file. On startup it:

1. Registers its config schema with the `configuration` worker
   (`configuration::register`, id `storage`).
2. Fetches the live, env-expanded config (`configuration::get`).
3. Subscribes to `configuration:updated` events and hot-reloads.

`--config <path>` is an **optional seed**: when given, the file is loaded and
sent as `initial_value` the first time the schema is registered (no stored value
yet). It is not the live source of truth — once a value exists in the
`configuration` worker, that value wins.

### Hot-reload scope

On a `configuration:updated` event the worker re-fetches the authoritative
config from the `configuration` worker (it does **not** trust the event
payload). Every setting is live: bucket additions/removals, providers,
credentials, underlying names, notification sources, local data directory,
and the local HTTP listener/public URL all apply without restarting the worker.

Buckets are applied one by one. A bucket whose backend cannot be built — a
`secret://` reference that does not resolve, a malformed field, a credentials
JSON that does not parse — stays listed by `storage::listBuckets` with
`error` / `error_stage`, and every call on it answers `BUCKET_UNAVAILABLE
{bucket, stage, message}`; every other bucket keeps working. A failed bucket is
retried every 15 s (a secrets worker that came up late), and right away when a
secret it references changes. A reload rebuilds only the buckets whose entry or
resolved credentials changed: the others keep their clients, and notification
pollers whose source did not change keep running. A notification source that
cannot be prepared (a Cloudflare token the queue refuses) leaves its buckets
working without triggers and is reported as `notifications_error`.

The native-local listener stays transactional: the worker binds a new
`bind_address` before publishing anything, and a port that is taken rejects the
whole update, leaving the previous configuration serving requests.

A fresh install with no configured buckets runs with zero backends until a
bucket is configured.

### Config shape

Each bucket pins a `provider` (`s3` | `gcs` | `r2` | `local`) and the
credentials for that provider. Buckets without `notifications:` work fine for
RPCs; they just don't fire triggers.

```yaml
providers:
  local:
    data_dir: data/storage             # relative to III_COMPOSE_DIR
    http:
      bind_address: 0.0.0.0:49200      # omit `http` to disable direct transfers
      public_url: http://10.0.0.42:49200 # browser-visible LAN/VPN/proxy URL

buckets:
  uploads:
    provider: s3
    bucket: my-app-uploads             # underlying cloud bucket
    region: us-east-1
    notifications:
      sqs_queue_url: https://sqs.us-east-1.amazonaws.com/123/my-app-uploads-events

  documents:
    provider: gcs
    bucket: my-app-documents
    # credentials_file: /etc/iii/gcs-sa.json   # required for presignUrl

  avatars:
    provider: r2
    bucket: avatars
    account_id: env://R2_ACCOUNT_ID
    access_key_id: secret://STORAGE_AVATARS_ACCESS_KEY_ID
    secret_access_key: secret://STORAGE_AVATARS_SECRET_ACCESS_KEY
    display: { icon: image, color: "#F38020" }

  photos:
    provider: s3                       # S3-compatible: MinIO, B2, Wasabi, Spaces…
    region: us-east-1
    endpoint_url: https://minio.example.com
    force_path_style: true
    access_key_id: secret://STORAGE_PHOTOS_ACCESS_KEY_ID
    secret_access_key: secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY

  reports:
    provider: gcs
    credentials_json: secret://STORAGE_REPORTS_CREDENTIALS_JSON   # the service-account JSON itself

  scratch:
    provider: local
    bucket: scratch
    display:
      icon: hard-drive                 # a Lucide icon name
      color: "#3B82F6"                 # #RRGGBB
```

The map key (`uploads`) is the worker-facing bucket name handlers
reference: `[a-z0-9][a-z0-9_-]{0,62}`. The nested `bucket:` is the underlying
cloud bucket (default: the map key). They can differ. `display` is read only by
clients (the console, the app) to draw the bucket.

### Credentials and the secrets worker

Every credential field — `access_key_id`, `secret_access_key`,
`session_token`, r2's `account_id`, gcs's `credentials_json` and r2's
`notifications.api_token` — takes, besides a literal or a `${VAR}` template:

- `secret://NAME` — a value in the `secrets` worker's encrypted vault;
- `env://NAME` — an environment variable the `secrets` worker reads (the
  project's `.env`, else its own environment).

`NAME` matches `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`. The worker resolves them with
`secrets::resolve` as consumer **`storage`**: the secret must list `storage` in
its `consumers` (`secrets::set {name, value, consumers: ["storage"]}`), else the
bucket is unavailable with `error_stage: "secrets"` and a message saying so.
Resolved values never appear in the configuration, a reply, logs or traces,
and every error message is scrubbed of the credentials used.

Resolved values are cached. The worker binds the secrets worker's
`secrets::changed` trigger to its internal `storage::on-secret-change`: a
rotated, deleted or re-shared secret is re-read at once and only the buckets
that use it are rebuilt (deleted → those buckets report the error; re-created →
they come back). Without the event a cached value is re-read every 5 minutes,
and a failed one every 10–15 seconds.

`secret_access_key`, `session_token`, `credentials_json` and
`notifications.api_token` are **sensitive**: `storage::getBucket` shows their
literals as `"__redacted__"`, and `storage::saveBucket` / `storage::testBucket`
refuse a literal inside `bucket` (it goes as a reference or in `credentials`).
Literals and `${VAR}` templates that are already stored keep working.

### Per-provider notes

- **S3** — defaults to the AWS credential chain (env, `~/.aws`, IMDS,
  IRSA). Override with `access_key_id` / `secret_access_key` /
  `session_token` only if the default chain doesn't fit.
- **GCS** — defaults to ADC (`GOOGLE_APPLICATION_CREDENTIALS`, GCE
  metadata, `gcloud auth application-default login`). `credentials_json`
  (the service-account JSON itself, usually `secret://…`) takes precedence over
  `credentials_file` (a path on the engine host). `presignUrl` requires a
  service-account key (or the metadata server's identity on Google Cloud);
  without one, or with an `endpoint_url` override, GCS presigns return
  `PRESIGN_UNSUPPORTED` and `listBuckets` reports `direct_downloads: false`.
  Pub/Sub notifications still authenticate through ADC.
- **R2** — required: `account_id`, `access_key_id`, `secret_access_key`.
  Endpoint URL is derived automatically as
  `https://{account_id}.r2.cloudflarestorage.com`.
- **local** — implemented directly by the worker. Object bytes and JSON
  metadata are persisted under `data_dir`; no sidecar or external binary is
  required. `http` is optional: omit it when only the small inline RPCs are
  needed. When enabled, `bind_address` controls the listener and `public_url`
  controls the base URL returned to clients. Set `public_url` whenever clients
  reach the worker through a LAN address, VPN, container port mapping, or
  reverse proxy. With port `0` and no `public_url`, the worker chooses a free
  loopback port and returns that address. The native listener includes browser
  CORS support for these signed transfers; cloud buckets need equivalent CORS
  rules on the provider when uploads originate in the Console.

> Existing rustfs data directories are not imported automatically: the native
> backend uses its own object/metadata layout under `data_dir`. Export or copy
> existing objects before upgrading, then upload them into the native bucket.

### Custom endpoints

S3, R2, and GCS bucket configs accept an optional `endpoint_url` field
for self-hosted S3-compatible stores (MinIO, Ceph, SeaweedFS), staging
environments, or local testing against fake-gcs-server.

```yaml
buckets:
  scratch-self-hosted:
    provider: s3
    region: us-east-1
    endpoint_url: https://s3.internal.example.com
    bucket: scratch
```

R2 with `endpoint_url` set emits a `tracing::warn!` at startup — the
field is fully functional but production R2 should omit it and let the
worker derive the endpoint automatically.

### Wiring notifications

| Provider | Config field(s) | Setup |
|---|---|---|
| S3 | `notifications.sqs_queue_url` | SQS queue + bucket event config for `s3:ObjectCreated:*` / `s3:ObjectRemoved:*` + `sqs:ReceiveMessage,DeleteMessage` IAM on the queue ARN. |
| GCS | `notifications.pubsub_subscription` | `gsutil notification create -t TOPIC -e OBJECT_FINALIZE,OBJECT_DELETE gs://<bucket>` + `roles/pubsub.subscriber` on the subscription. |
| R2 | `notifications.queue_id` + `notifications.api_token` | Cloudflare Queue + R2 event notifications on the bucket + API token scoped `queue:consume`. |
| local | (none) | The worker dispatches events immediately after a native local write/delete commits. |

Other config keys and their defaults live in
[`src/config.rs`](src/config.rs); wire-stable error codes returned by
every RPC live in [`src/error.rs`](src/error.rs).

## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `storage::object-created` | An object is written (PUT, multipart complete, copy-in). | `{ bucket, key, size, content_type, etag, version_id?, event_time }` |
| `storage::object-deleted` | An object (or version, on versioned buckets) is removed. | `{ bucket, key, version_id?, event_time }` |

The worker itself binds two internal functions (metadata `internal: true`,
denied to agents): `storage::on-config-change` to the configuration worker's
`configuration:updated` for its entry, and `storage::on-secret-change` to the
secrets worker's `secrets::changed` (the binding waits for the secrets worker
when it is not running yet). Both ignore their payload's values and re-read the
authoritative state.

Delivery is at-least-once. Handlers must return `{ ack: true }`; `false`,
panic, or timeout (`handler_timeout_ms`, default 60 s) leaves the
message in the upstream queue for redelivery.

```yaml
triggers:
  - type: storage::object-created
    config:
      bucket: uploads
      # event_types: [ObjectCreated:Put, ObjectCreated:CompleteMultipartUpload]   # optional filter
      # handler_timeout_ms: 60000

  - type: storage::object-deleted
    config:
      bucket: uploads
```

> **R2 trigger v1 caveat:** the Cloudflare Queues consume-from-outside
> REST API is the youngest of the four upstreams. The worker probes the
> consume endpoint at startup and surfaces `CF_QUEUE_AUTH_FAILED` for
> 401/403, so token misconfiguration is visible immediately. If you hit
> redelivery or auth-scope edge cases in production, file an issue —
> v1.1 will finalize the consume path.

## Console explorer

When the worker is connected, it injects a `storage` page into the Console.
Wide panels use three simultaneous levels—buckets, folders/objects, and object
details—while narrow panels show one level at a time. The explorer supports
folder breadcrumbs, pagination, metadata inspection, direct upload, signed
download, copy-key, and delete actions. Uploads and downloads use the direct
transfer endpoints instead of moving file bodies through an inline RPC.

For native local uploads, enable `providers.local.http`. Cloud uploads also
require the provider bucket to permit the Console origin through CORS.

The page's **configure** action opens a storage-specific editor while the
Console continues to own validation, dirty tracking, save, reset, and the
unsaved-change guard. The editor separates native-local runtime settings from
bucket mappings and presents only the fields relevant to Local, S3, GCS, or
R2. Saved changes are applied live; no storage setting requires a worker
restart.

## Bucket management

Bucket editors (the console, the iii app) create and change buckets through
four functions instead of rewriting the whole configuration. All four are
denied to agents by `iii-permissions.yaml` (they rewrite which providers and
credentials the worker uses, or send credentials where a candidate points).
Their requests refuse unknown keys and wrong-typed fields with `CONFIG_ERROR`
naming the field (keys starting with `_` are the engine's own metadata and are
ignored).

### Where credentials may go

A save makes the live bucket send its credentials to its endpoint; a test sends
them at once. Neither may become a way to read a credential the caller cannot
read (the vault never returns values, stored literals come back
`"__redacted__"`, files on the engine host are not readable through the bus):

- A value kept from the stored entry — a `"__redacted__"` restored, a `${VAR}`
  template left unchanged — only goes to the same provider and destination (the
  `endpoint_url` as scheme, host and port; else AWS, Google, or the R2 account).
  Moving the endpoint, account or provider needs the value entered again.
- A `secret://` / `env://` reference or a `credentials_file` that a stored bucket
  uses, in any credential field, only goes in the same field to a destination a
  stored bucket sends it to (entries that do not parse count too). `testBucket`
  counts every stored bucket, and so does `saveBucket`, except that a bucket
  moved to a new provider, endpoint or account takes its own `access_key_id`,
  `secret_access_key` and `account_id` along in the same fields when no other
  bucket shares them (only an identifier and a signature leave; the access key
  id follows even as a `secret://` reference because it is an identifier sent in
  clear in every SigV4 request — storing it as a secret keeps it out of the
  configuration, not out of requests). Its `session_token`, `credentials_json`
  and `credentials_file`, sent in clear or as bearer credentials, must be
  entered again, and a value another bucket shares (a Duplicate) needs a new
  one. An r2 API token always goes to Cloudflare's API, so it follows. A value
  never changes field. A Duplicate to the same endpoint keeps working.
- A `credentials_file` is a file of the engine host: a regular JSON key file of
  at most 64 KiB (never a FIFO or device), of type `service_account` or
  `authorized_user`, whose `token_uri` is Google's; one no stored bucket uses
  only goes to Google. Every failure gives the same message, including a key the
  auth library or Google refuses.
- The engine host's ambient credentials — the AWS default chain for an s3 bucket
  without keys, Google ADC for a gcs bucket without credentials — only go to the
  provider itself or to an endpoint a stored bucket already uses them with.
- A request may not introduce a `${VAR}` template (it reads the engine's
  environment, which nothing gates; `env://NAME` goes through the secrets
  worker's access rules), nor userinfo in `endpoint_url`. An r2 `account_id`
  must be a 32-hex account id before it becomes a hostname.

`credentials` stores its values as literals in the configuration, where
`configuration::get {raw: true}` shows them; the secrets vault is the intended
store.

### `storage::listBuckets`

`{}` → `{ buckets: [...], count, config_id }`, sorted by name. Each bucket:
`name`, `provider`, `bucket` (the provider-side bucket), `region?` (s3),
`endpoint_url?` (userinfo stripped), `account_id?` (r2: the literal or the
reference as stored), `display?`, `uses_secrets`, `direct_downloads` (whether a
presigned GET works for it: false for gcs without signing credentials or with
an `endpoint_url` override), and for a bucket that could not be built
`error` + `error_stage` (`config` | `secrets` | `auth`). Stored entries that do
not parse are listed too (`error_stage: "config"`).

### `storage::getBucket`

`{ name }` → `{ name, bucket }`: the entry as stored (`${VAR}` templates and
references unexpanded), sensitive literals and keys that are not fields as
`"__redacted__"` (a save keeps the stored value of either when the placeholder
comes back).

### `storage::saveBucket`

```json
{
  "name": "photos",
  "previous_name": "photos",
  "bucket": {
    "provider": "s3", "region": "us-east-1", "bucket": "my-photos",
    "endpoint_url": "https://minio.example.com", "force_path_style": true,
    "access_key_id": "secret://STORAGE_PHOTOS_ACCESS_KEY_ID",
    "secret_access_key": "secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY",
    "display": { "icon": "image", "color": "#3B82F6" }
  },
  "credentials": { "session_token": "…" },
  "create_bucket": true
}
```

→ `{ name, saved: true, created_bucket?, error?, error_stage? }`.

- No `previous_name` creates (a taken name is refused); `previous_name == name`
  updates; another `previous_name` renames (a taken name is refused). A rename
  of an entry without `bucket` keeps pointing at the same provider-side bucket.
- Keys that are not fields of the provider are refused by name, and so are
  literal sensitive values inside `bucket`. `credentials` (`access_key_id`,
  `secret_access_key`, `session_token`, `credentials_json`,
  `notifications_api_token`) is merged over `bucket` — trace redaction hides a
  key named `credentials` whole. `"__redacted__"` keeps the stored value.
- The save is a read-modify-write of the whole raw value through
  `configuration::set`: `providers`, the other buckets and their templates and
  references are written back untouched. `configuration::set` has no
  compare-and-set, but answers the value it replaced: when another writer got in
  between, the edit is applied again on top of that writer's value (best
  effort). The answer comes after the runtime reloaded; `error` /
  `error_stage` is the saved bucket's build error (the entry is saved anyway).
  Refusals are `CONFIG_ERROR`.
- Changing a local bucket's `bucket` points it at another directory under
  `data_dir/buckets/`; objects are not moved.
- Known limit: a writer that rewrites the whole value without re-applying (the
  console's configuration editor, a hand `configuration::set`) and read before
  a save overwrites it. Only a compare-and-set in `configuration::set` can fix
  that.
- `create_bucket: true` (s3, r2, S3-compatible) creates the provider-side
  bucket when it does not exist (`created_bucket: true`; already there →
  `false`), sending the region as `LocationConstraint` except for AWS's
  `us-east-1` and r2. A failure is reported in `error` with the entry saved.
  Local buckets always create their directory.

### `storage::deleteBucket`

`{ name }` → `{ deleted }`. Removes the entry only — never objects, local data
or secrets. Removing the last bucket is allowed.

### `storage::testBucket`

`{ bucket?, name?, credentials?, timeout_ms? }` →
`{ ok, provider, latency_ms, message?, stage?, bucket_missing?, direct_downloads? }`.

Builds a throwaway client (one attempt, no retries) with every reference
resolved fresh — never from the cache — and lists one object at the root.
Failures are data, not errors: `stage` is `config` (the entry), `secrets` (a
reference did not resolve), `endpoint` (DNS, connect, TLS, timeout), `auth`
(rejected keys or signature, no credentials at all), `bucket` (NoSuchBucket,
with `bucket_missing: true`; another region) or `list` (anything else).
`timeout_ms` defaults to 10000 (max 30000). Without `bucket` it tests the
stored bucket `name`; with both, `name` supplies the `"__redacted__"` values and
unchanged `${VAR}` templates — and those values, like a reference a stored bucket
already uses, are only sent to that bucket's endpoint. A local bucket checks
that its directory can be written.

## RPC reference notes

### Inline object calls are intentionally limited

`storage::putObject` and `storage::getObject` are convenience functions for
small configuration fragments, thumbnails, test fixtures, and similar values.
They are not streaming APIs: the full object crosses the iii engine as base64
and is held in memory. The decoded body is capped at 10 MiB. For files—even
when a file happens to fit below that cap—prefer direct transfer endpoints:

- upload: `storage::presignPost` for local multipart POST, or
  `storage::presignUrl` with `method: "PUT"`;
- download: `storage::presignUrl` with `method: "GET"`.

### `storage::getObject` — ranged reads

`offset` (default 0) and `length` (default: to the end) read a byte range of
any object; the response adds `offset` and `length` (the bytes returned) while
`size` stays the object's total size. The 10 MiB cap applies to the bytes
returned, so `{ offset, length: 2097152 }` chunks read a 2 GB video. An
`offset` equal to the size returns an empty body; past it, `INVALID_RANGE
{offset, size}`. S3 and R2 send a `Range` GET, GCS a ranged download pinned to
the generation it read, local storage seeks.

### `storage::presignPost` — multipart upload

Returns `{ url, fields, expires_at }`. Add every returned field to a
`multipart/form-data` body before the `file` part. Native local storage streams
the file to disk and can enforce optional `max_size_bytes` without buffering.
Other providers currently return `PRESIGN_UNSUPPORTED`; use a signed PUT there.

### `storage::presignUrl` — GET-only response-override params

Two optional fields are accepted only when `method` is `"GET"`. Passing
either on a `PUT` presign returns `INVALID_PRESIGN_PARAMS`.

| Field | Type | Description |
|---|---|---|
| `response_content_disposition` | `string` (optional) | Override `Content-Disposition` header on the served response (e.g. `"attachment; filename=\"report.pdf\""`). |
| `response_content_type` | `string` (optional) | Override `Content-Type` header on the served response (e.g. `"application/pdf"`). |

```ts
const { url } = await iii.trigger({
  function_id: 'storage::presignUrl',
  payload: {
    bucket: 'uploads',
    key: 'reports/q1.pdf',
    method: 'GET',
    expires_in_seconds: 300,
    response_content_disposition: 'attachment; filename="q1.pdf"',
    response_content_type: 'application/pdf',
  },
})
```

### Local signed downloads: Range and HEAD

The native local download route answers `HEAD` and honours `Range`, so a video
player can seek: `Accept-Ranges: bytes` on every answer; `bytes=a-b`, `bytes=a-`
and `bytes=-n` → `206 Partial Content` with `Content-Range`; a range that starts
at or past the end, `bytes=-0`, or any range of an empty object → `416` with
`Content-Range: bytes */size`. A request for several ranges (`bytes=0-1,5-6`),
another unit, or a malformed or inverted range is answered with the whole
object (`200`), which RFC 9110 allows; `If-Range` naming another version does
the same. Downloads carry `X-Content-Type-Options: nosniff` and
`Content-Security-Policy: sandbox`, so an HTML object never runs as a page of
the listener's origin. The signing key is per worker process: URLs issued
before a restart answer `401`. `presignUrl` refuses control characters in
`response_content_disposition` / `response_content_type`.

## Local development & testing

The committed `config.yaml` declares a single `scratch` bucket served directly
by the worker. Pass it as a seed so the `configuration` worker picks it
up on first boot — zero cloud credentials required.

```bash
# In one terminal: start the engine (must include the configuration worker)
iii --config config.yaml

# In another: build & run the worker, seeding config.yaml on first registration
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

The worker registers its schema with the `configuration` worker (seeding
`config.yaml` if no stored value exists), fetches the live config, initializes
the native store and optional direct-transfer HTTP listener, and registers
`storage::putObject`, `storage::getObject`, `storage::deleteObject`,
`storage::presignUrl`, `storage::presignPost`, `storage::headObject`,
`storage::listBuckets`, `storage::listObjects`, `storage::getBucket`,
`storage::saveBucket`, `storage::deleteBucket` and `storage::testBucket`. Files land under
`data/storage/` under `III_COMPOSE_DIR` (configurable via
`providers.local.data_dir`).

Running `--manifest` prints the registry-publish JSON without touching the
engine — useful when testing CI flows:

```bash
cargo run -- --manifest | jq .
```

### Tests

```bash
cargo test --lib                # unit tests (config, manifest, handlers, triggers)
cargo test --test schemas       # schema regression for every `storage::*` RPC
cargo test --test manifest      # `--manifest` subprocess contract
cargo test --test integration   # spec §9 pattern A: spawns engine + worker
```

`tests/integration.rs` self-skips when the `iii` engine is not available on
`PATH`. Native local tests require no external storage dependency. The richer
per-provider e2e suite under `tests/e2e/`
is env-var-gated — see `tests/e2e/run-tests.sh` for the orchestrator. Its
harness covers bucket management, ranged reads and HTTP Range against every
selected provider (`--providers=local` needs nothing; `all` adds MinIO through
docker compose).

### Verification before publishing

The full preflight checklist for binary workers
([`docs/sops/binary-worker.md`](https://github.com/iii-hq/workers/blob/main/docs/sops/binary-worker.md) §11):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./target/debug/storage --manifest | jq .
```
