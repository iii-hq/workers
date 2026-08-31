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
> Both enforce a 10 MiB decoded hard limit, but they are intended for truly
> small values, not general file transfer. Use `presignPost`/a signed PUT for
> uploads and a signed GET from `presignUrl` for downloads.

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

Application is transactional. The worker first prepares the candidate
backends, notification clients, local service generation, and—when the bind
address changes—the new TCP listener. Only after those fallible steps succeed
does it publish the new runtime and gracefully retire the old listener and
pollers. A failed backend build, authentication probe, or port bind leaves the
previous configuration serving requests.

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
    account_id: ${R2_ACCOUNT_ID}
    access_key_id: ${R2_ACCESS_KEY_ID}
    secret_access_key: ${R2_SECRET_ACCESS_KEY}

  scratch:
    provider: local
    bucket: scratch
```

The map key (`uploads`) is the worker-facing bucket name handlers
reference; the nested `bucket:` is the underlying cloud bucket. They can
differ.

### Per-provider notes

- **S3** — defaults to the AWS credential chain (env, `~/.aws`, IMDS,
  IRSA). Override with `access_key_id` / `secret_access_key` /
  `session_token` only if the default chain doesn't fit.
- **GCS** — defaults to ADC (`GOOGLE_APPLICATION_CREDENTIALS`, GCE
  metadata, `gcloud auth application-default login`). `presignUrl`
  requires a service-account JSON with a private key — supply
  `credentials_file` explicitly when running on metadata-server-only
  sources (e.g., GKE Workload Identity), otherwise GCS presigns return
  `PRESIGN_UNSUPPORTED`.
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
`storage::listBuckets`, and `storage::listObjects`. Files land under
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
is env-var-gated — see `tests/e2e/run-tests.sh` for the orchestrator.

### Verification before publishing

The full preflight checklist for binary workers
([`docs/sops/binary-worker.md`](https://github.com/iii-hq/workers/blob/main/docs/sops/binary-worker.md) §11):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./target/debug/storage --manifest | jq .
```
