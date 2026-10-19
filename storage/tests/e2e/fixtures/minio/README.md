# MinIO E2E fixture

The public `quay.io/minio/minio` and `quay.io/minio/mc` images used by this
suite are no longer anonymously accessible. Build the fixture from the
same upstream releases instead of requiring registry credentials or dropping
S3/R2 and notification coverage.

Pinned sources:

- MinIO `RELEASE.2024-12-18T13-15-44Z`:
  `16f8cf1c52f0a77eeb8f7565aaf7f7df12454583`
- mc `RELEASE.2024-11-21T17-21-54Z`:
  `1681e4497c09d7438a34e846f76dbde972ab7daf`

The multi-stage Dockerfile builds both binaries with Go 1.23.12, preserves
their upstream licenses, and ships only the binaries and CA certificates
in the runtime image. Source dependencies remain governed by upstream
`go.mod`/`go.sum`.

`docker compose --profile cloud up -d` builds this service automatically.
`minio-init.sh` invokes the bundled mc inside the running service, so it
never pulls a separate client image. The credentials are isolated fixture
values, not production secrets.
