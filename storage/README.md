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

`storage` reads one `config.yaml` describing one or more buckets. Each
bucket pins a `provider` (`s3` | `gcs` | `r2` | `local`) and the
credentials for that provider. Buckets without `notifications:` work
fine for RPCs; they just don't fire triggers.

```yaml
workers:
  - name: storage
    config:
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

## Local development & testing

The committed `config.yaml` declares a single `scratch` bucket served by the
bundled rustfs sidecar, so you can run the worker against a local engine with
zero credentials.

```bash
# In one terminal: start the engine
iii start

# In another: build & run the worker
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

The worker spawns a rustfs process on a random port, waits for it to become
healthy, then registers `storage::putObject`, `storage::getObject`,
`storage::deleteObject`, and `storage::presignUrl`. Files land under
`./data/storage/` (configurable via `providers.local.data_dir`).

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
(`workers/binary-worker.md` §11):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./target/debug/storage --manifest | jq .
```
