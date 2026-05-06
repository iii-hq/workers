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
