# storage

Object storage for the iii engine over S3, GCS, R2, and a managed local
backend. Streamed uploads, presigned URLs, and `object-created` /
`object-deleted` triggers — all behind one `bucket:` name regardless of
the cloud underneath.

## Install

```bash
iii worker add storage
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Quickstart

Upload a profile photo, hand the browser a presigned URL for the next
upload, then read it back.

```ts
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')

await iii.trigger({
  function_id: 'storage::putObject',
  payload: {
    bucket: 'uploads',
    key: 'u/1/profile.jpg',
    body_base64: fileBase64,         // ≤ 10 MiB inline; use presignUrl above that
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
payload). It then rebuilds the in-memory **backend map only**, and only when the
bucket/notification **topology is unchanged** — i.e. when just backend
connection settings changed (credentials, endpoint, path-style).

Any change to the bucket set, a bucket's provider or underlying name, a
notification source, or the local rustfs data dir is **refused**: the worker
keeps the previously-running backends and logs that a restart is required.
This avoids a split-brain where RPC reads/writes move to a new backend while
the notification pollers/webhook stay wired to the old topology. A failed
rebuild likewise keeps the previous backends.

A fresh install with no configured buckets runs with zero backends until a
bucket is configured.

### Config shape

Each bucket pins a `provider` (`s3` | `gcs` | `r2` | `local`) and the
credentials for that provider. Buckets without `notifications:` work fine for
RPCs; they just don't fire triggers.

```yaml
providers:
  local:
    data_dir: ./data/storage           # rustfs sidecar root

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
- **local** — managed [rustfs](https://github.com/rustfs/rustfs) sidecar,
  spawned only when at least one `provider: local` bucket is configured.
  Discovery order: `$RUSTFS_BIN`, then `./rustfs` next to the worker
  binary, then `rustfs` on `$PATH`. Operators install rustfs separately
  for now (v1.1 will side-download a pinned release).

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
| local | (none) | Worker spawns rustfs and wires its notify webhook to a loopback HTTP receiver automatically. |

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

## RPC reference notes

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

The committed `config.yaml` declares a single `scratch` bucket served by the
bundled rustfs sidecar. Pass it as a seed so the `configuration` worker picks it
up on first boot — zero cloud credentials required.

```bash
# In one terminal: start the engine (must include the configuration worker)
iii start

# In another: build & run the worker, seeding config.yaml on first registration
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

The worker registers its schema with the `configuration` worker (seeding
`config.yaml` if no stored value exists), fetches the live config, then spawns a
rustfs process on a random port, waits for it to become healthy, and registers
`storage::putObject`, `storage::getObject`, `storage::deleteObject`,
`storage::presignUrl`, and `storage::headObject`. Files land under
`./data/storage/` (configurable via
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

`tests/integration.rs` self-skips when `iii` (engine) or `rustfs` is not
available on `PATH` (or via `$RUSTFS_BIN`), so CI hosts without those
dependencies still pass. The richer per-provider e2e suite under `tests/e2e/`
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
