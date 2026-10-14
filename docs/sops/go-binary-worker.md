# Go binary worker

Go workers publish standalone binaries with `artifact.kind: go-binary` and the
same archive, checksum, and Registry binary contract as Rust workers. Start with
the shared checklist in [`new-worker.md`](new-worker.md).

## Version and toolchain

Keep the release metadata version in `<worker>/package.json`:

```json
{
  "name": "<worker>",
  "version": "0.1.0",
  "private": true
}
```

This file owns only package identity and version. It does not require Node,
package installation, or membership in the pnpm UI workspace. `go.mod` and
`go.sum` own the Go module and dependency checksums; the `go` directive selects
the minimum toolchain, never the worker release version. Pin the build toolchain
in the private catalog to the version used by `go.mod`.

The public manifest declares `language: go`, `deploy: binary`,
`manifest: package.json`, and `bin: <worker>`. Its dependencies retain the
repository's `latest` runtime dependency policy.

```yaml
workers:
  <worker>:
    source:
      path: <worker>
      package_manifest: package.json
    artifact:
      kind: go-binary
      toolchain:
        name: go
        version: 1.27.1
      binary: <worker>
      targets:
      - aarch64-apple-darwin
      - x86_64-apple-darwin
      - x86_64-unknown-linux-gnu
      - aarch64-unknown-linux-gnu
      windows_exception: Explain the worker-specific Windows limitation.
    publish: true
```

Only declare platforms the worker can compile and support. WhatsApp uses this
four-target Unix matrix because its session and inbox locking depend on POSIX
`flock` and `O_NOFOLLOW`; it does not publish Windows binaries. The compiler
maps the Registry triples to Go's `GOOS` and `GOARCH` through
[`deployment_targets.py`](../../.github/scripts/deployment_targets.py).

## Build and capture contract

Declare a string `version` variable in package `main`. Release builds run
`go build -mod=readonly -trimpath -ldflags "-s -w -X main.version=<metadata-version>"`
with `CGO_ENABLED=0`, `GOWORK=off`, and `GOTOOLCHAIN=local`. They require no C
cross-compiler. Compiled frontend assets must be committed and embedded with
`go:embed`; Go release builds do not install or build frontend dependencies.

Each build unit produces `<binary>-<target>.tar.gz` and a SHA-256 sidecar.
Windows-capable Go workers use a ZIP containing `<binary>.exe`. The prepared
inventory preserves `role: binary`, and publication projects these archives
to the existing Registry `type: binary` and `binaries` map. Release Control
still selects the published version independently from source metadata.

Go workers must support `--local-config --collect-interface` for registration
capture. This mode registers typed functions and triggers against `III_URL`
without initializing storage, sessions, or external backends. A committed
`config.collect.yaml`, when present, is hashed into the descriptor and passed
through `--config` from the selected immutable source. Both PR capture and
release capture use these flags; release capture runs only the prepared Linux
amd64 binary.

## Checks

Go's adjacent `*_test.go` files satisfy the repository's tests requirement;
a separate `tests/` directory is optional. CI discovers `go-binary` artifacts
in the Go matrix and runs formatting, vet, readonly tests, and the release build:

```bash
cd <worker>
test -z "$(gofmt -l .)"
CGO_ENABLED=0 GOWORK=off go vet -mod=readonly ./...
CGO_ENABLED=0 GOWORK=off go test -mod=readonly ./...
make build
```

Check every declared cross-compilation target before adding it to the catalog.
Cross-compilation establishes build compatibility; behavior that depends on the
operating system also needs native platform validation.
