# storage

> Object storage abstraction over S3, GCS, R2, and a managed local backend. Streamed uploads, presigned URLs, and change notifications.

| field | value |
|-------|-------|
| version | 2.1.0 |
| type | binary |
| supported_targets | x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu |
| author | iii |

## Install

```sh
iii worker add storage@2.1.0
```

## Configure

`storage` reads a single `config.yaml` describing one or more buckets. Each bucket pins a `provider` (one of `s3`, `gcs`, `r2`, `local`) and the credentials/endpoint details for that provider. Buckets without notifications wired up don't emit triggers; functions still work.

```yaml
workers:
  - name: storage
    config:
      providers:
        local:
          data_dir: ./data/storage

      buckets:
        uploads:
          provider: s3
          bucket: my-app-uploads
          region: us-east-1
          notifications:
            sqs_queue_url: https://sqs.us-east-1.amazonaws.com/123/my-app-uploads-events

        documents:
          provider: gcs
          bucket: my-app-documents
          # credentials_file: /etc/iii/gcs-sa.json    # required for presignUrl

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

The map key (e.g. `uploads`) is the worker-facing bucket name — what handlers reference. The nested `bucket:` is the underlying cloud bucket name. They can differ.

### Per-provider notes

**S3** — defaults to the AWS credential chain (env, `~/.aws`, IMDS, IRSA). Override with `access_key_id` / `secret_access_key` / `session_token` only if the default chain doesn't fit.

**GCS** — defaults to ADC (`GOOGLE_APPLICATION_CREDENTIALS`, GCE metadata, `gcloud auth application-default login`). For `presignUrl` to work the credentials **must** include a private key (a service-account JSON), not metadata-server-only credentials. GKE Workload Identity users without an SA key get `PRESIGN_UNSUPPORTED`.

**R2** — required: `account_id`, `access_key_id`, `secret_access_key`. The worker constructs the endpoint URL `https://{account_id}.r2.cloudflarestorage.com` automatically.

**local** — managed [rustfs](https://github.com/rustfs/rustfs) sidecar. Spawned only when at least one `provider: local` bucket is configured. Discovery order: `$RUSTFS_BIN`, then `./rustfs` next to the worker binary, then `rustfs` on `$PATH`. Operators install rustfs separately for now (v1.1 will side-download a pinned release).

### Custom endpoints (`endpoint_url`)

S3, R2, and GCS bucket configs accept an optional `endpoint_url` field. Use it for self-hosted S3-compatible stores (MinIO, Ceph, SeaweedFS), staging environments, or local testing against fake-gcs-server.

```yaml
buckets:
  scratch-self-hosted:
    provider: s3
    region: us-east-1
    endpoint_url: https://s3.internal.example.com
    bucket: scratch
```

R2 with `endpoint_url` set emits a `tracing::warn!` at startup — the field is fully functional but most production R2 deployments should omit it and let the worker derive `https://<account_id>.r2.cloudflarestorage.com` automatically.

## Functions

```ts
import { call } from 'iii-sdk'

await call('storage::putObject', {
  bucket: 'uploads',
  key: 'u/1/profile.jpg',
  body_base64: fileBase64,
  content_type: 'image/jpeg',
})

const { body_base64 } = await call('storage::getObject', {
  bucket: 'uploads',
  key: 'u/1/profile.jpg',
})

const { url, expires_at } = await call('storage::presignUrl', {
  bucket: 'uploads',
  key: 'u/1/profile.jpg',
  method: 'PUT',
  expires_in_seconds: 600,
  content_type: 'image/jpeg',
})

await call('storage::deleteObject', { bucket: 'uploads', key: 'u/1/profile.jpg' })
```

| Function | Purpose |
|---|---|
| `storage::putObject` | Write an object. Body is base64; max **10 MiB inline**. Larger payloads use `presignUrl`. |
| `storage::getObject` | Read an object. Body is base64; max **10 MiB**. Larger reads use `presignUrl`. |
| `storage::deleteObject` | Idempotent delete. Returns `{deleted: false}` if the object didn't exist. |
| `storage::presignUrl` | Issue a short-lived signed URL the browser can use to PUT or GET directly. `expires_in_seconds ∈ [30, 86400]`. PUT presigns require `content_type` (pinned into the signature to prevent type smuggling). |

## Triggers

Two trigger types: `storage::object-created` and `storage::object-deleted`. Both share an at-least-once delivery model — handlers must return `{ ack: true }` on success; `false`, panic, or timeout (default 60s) leave the message in the queue for redelivery.

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

The trigger references a `bucket:` from the buckets map. Wiring depends on the bucket's provider:

### S3

1. Create an SQS queue. Note the queue URL.
2. On the S3 bucket: configure event notifications for `s3:ObjectCreated:*` and `s3:ObjectRemoved:*` → that queue.
3. Grant the worker's IAM principal `sqs:ReceiveMessage`, `sqs:DeleteMessage` on the queue ARN.
4. Set `notifications.sqs_queue_url` on the bucket config.

### GCS

1. Create a Pub/Sub topic and subscription. Note the subscription name (`projects/X/subscriptions/Y`).
2. `gsutil notification create -t TOPIC -e OBJECT_FINALIZE,OBJECT_DELETE gs://my-app-documents`.
3. Grant the worker's GCP service account the `roles/pubsub.subscriber` role on the subscription.
4. Set `notifications.pubsub_subscription` on the bucket config.

### R2

1. Create a Cloudflare Queue. Note the queue ID.
2. Enable R2 event notifications on the bucket → that queue.
3. Create a Cloudflare API token scoped to `queue:consume`.
4. Set `notifications.queue_id` and `notifications.api_token` on the bucket config.

> **R2 trigger v1 caveat:** R2 trigger ships in v1.0 as soft-ship. The Cloudflare Queues consume-from-outside REST API is the youngest of the four. If you hit redelivery or auth-scope edge cases, file an issue — v1.1 will finalize the consume path.

### local

No setup. The worker spawns rustfs and wires its notify webhook target to a loopback HTTP receiver automatically.

## Errors

Returned `IIIError::Handler` bodies carry a stable `code` field:

| Code | Meaning |
|---|---|
| `CONFIG_ERROR` | Startup-only. Bad YAML, missing required field per provider, trigger references unknown bucket, etc. |
| `UNKNOWN_BUCKET` | `bucket` parameter doesn't match any configured bucket. |
| `OBJECT_NOT_FOUND` | Object missing on `getObject` (or `deleteObject` with explicit `version_id`). |
| `BODY_TOO_LARGE` | `putObject` decoded body exceeds 10 MiB cap. |
| `OBJECT_TOO_LARGE` | `getObject` object exceeds 10 MiB cap. |
| `INVALID_BASE64` | Body could not be decoded. |
| `INVALID_PRESIGN_PARAMS` | `expires_in_seconds` out of `[30, 86400]`, or PUT presign without `content_type`. |
| `PRESIGN_UNSUPPORTED` | GCS bucket configured with metadata-server-only credentials (no signing key). |
| `LOCAL_BACKEND_DOWN` | rustfs sidecar exited unexpectedly; manual worker restart required. |
| `LOCAL_BACKEND_BIN_NOT_FOUND` | `provider: local` configured but no rustfs binary discoverable. Fatal at startup. |
| `LOCAL_BACKEND_BOOT_FAILED` | rustfs binary found but did not become healthy within 30s. |
| `PROVIDER_ERROR` | Wraps the underlying provider error. Carries `provider` and `inner_code` (best-effort SDK code). |
| `PROVIDER_AUTH_FAILED` | Credentials missing/expired/scope-insufficient. Distinct from `PROVIDER_ERROR` so dashboards can alert separately. |
| `CF_QUEUE_AUTH_FAILED` | Cloudflare Queues consume probe failed. |
| `TRIGGER_DISPATCH_TIMEOUT` | Trigger handler exceeded `handler_timeout_ms`. Message not acked; redelivered. |

## Troubleshooting

- **`LOCAL_BACKEND_BIN_NOT_FOUND`** — set `RUSTFS_BIN` to an absolute path, or place a `rustfs` binary next to the `storage` binary, or install rustfs on `$PATH`.
- **GCS `PRESIGN_UNSUPPORTED`** — supply an explicit `credentials_file` pointing at a service-account JSON. ADC sources without a signing key (e.g., GKE Workload Identity) cannot generate V4 signatures.
- **R2 trigger silent** — verify the API token scope. The worker probes the consume endpoint at startup and surfaces `CF_QUEUE_AUTH_FAILED` for 401/403 responses.
- **`BODY_TOO_LARGE` / `OBJECT_TOO_LARGE`** — switch to `presignUrl` for direct browser-to-cloud PUT/GET. The 10 MiB cap is intentional: it bounds per-RPC memory and keeps the function envelope predictable.

## End-to-end test harness

`tests/e2e/workers/harness/` contains a TypeScript harness that drives `storage` as a real engine-managed worker — exercising RPC handlers, the rustfs sidecar lifecycle, and `object-created` / `object-deleted` trigger dispatches end-to-end. It complements the existing Rust integration tests under `tests/e2e/*.rs`, which call the library directly.

Run it:

```sh
./tests/e2e/run-tests.sh
```

The script builds the worker (`cargo build --release --bin storage`), starts the engine, launches the harness as a host node process, greps for a `HARNESS_DONE: PASS|FAIL N/N` sentinel, and prints a per-case report. Local backend only — no AWS/GCP/CF credentials needed. Pass `--filter=<substring>` to run a subset, `--keep` to retain `tests/e2e/data/`, or `--no-build` to skip the cargo step.

## License

MIT.
