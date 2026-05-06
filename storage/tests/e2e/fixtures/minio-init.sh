#!/usr/bin/env bash
# Idempotent bucket bootstrap for the MinIO container. Called from
# run-tests.sh after `docker compose up` reports healthy. Creates the
# two buckets the harness uses (scratch-s3, scratch-r2) via the AWS
# CLI's S3 protocol; no `mc` install required.
set -euo pipefail

ENDPOINT="${MINIO_ENDPOINT:-http://127.0.0.1:9000}"
AK="${MINIO_ACCESS_KEY:-minioadmin}"
SK="${MINIO_SECRET_KEY:-minioadmin}"

# Use the awscli image instead of requiring it on the host. ~30MB pull,
# cached after the first run.
aws_s3() {
  docker run --rm --network host \
    -e AWS_ACCESS_KEY_ID="$AK" \
    -e AWS_SECRET_ACCESS_KEY="$SK" \
    -e AWS_DEFAULT_REGION=us-east-1 \
    amazon/aws-cli:2.17.22 \
    --endpoint-url "$ENDPOINT" \
    "$@"
}

for bucket in scratch-s3 scratch-r2; do
  if aws_s3 s3api head-bucket --bucket "$bucket" 2>/dev/null; then
    echo "[minio-init] bucket $bucket already exists"
  else
    aws_s3 s3api create-bucket --bucket "$bucket" >/dev/null
    echo "[minio-init] created bucket $bucket"
  fi
done

# Bind the BRIDGE webhook target (declared via MINIO_NOTIFY_WEBHOOK_*
# env vars on the MinIO container) to scratch-s3 for create/delete events.
# `mc event add` is idempotent in spirit but errors if the rule already
# exists — we tolerate that by checking `mc event list` first.
mc() {
  docker run --rm --network host \
    -e MC_HOST_local="http://${AK}:${SK}@127.0.0.1:9000" \
    minio/mc:RELEASE.2024-11-21T17-21-54Z \
    "$@"
}

WEBHOOK_ARN="arn:minio:sqs::BRIDGE:webhook"

# Wait briefly for MinIO admin to recognise the env-supplied target.
for _ in 1 2 3 4 5; do
  if mc admin info local >/dev/null 2>&1; then break; fi
  sleep 1
done

if mc event list local/scratch-s3 2>/dev/null | grep -qF "$WEBHOOK_ARN"; then
  echo "[minio-init] event rule already bound on scratch-s3"
else
  if mc event add local/scratch-s3 "$WEBHOOK_ARN" --event put,delete >/dev/null; then
    echo "[minio-init] bound $WEBHOOK_ARN → scratch-s3 (put,delete)"
  else
    echo "[minio-init] WARN: could not bind webhook event rule on scratch-s3" >&2
    # Don't hard-fail — non-trigger e2e cases should still run; the trigger
    # cases will skip via the runner's per-provider trigger probe.
  fi
fi
