# Develop the WhatsApp worker

Build, inspect, and test the worker from source. For installation and first use, follow the [consumer README](../README.md).

The dependency is pinned to
`8b41cfe6d9c4` (`v0.0.0-20260929112325-8b41cfe6d9c4`). The iii Go SDK is pinned
to merged commit `a075c6e40389` (`v0.24.6-rc.1.0.20261002194513-a075c6e40389`),
which includes the trigger schema API. A local SDK workspace is optional for SDK
development.

This is a native function API, not an HTTP-compatible deployment of OpenWA.
[openwa-rust-coverage.json](../openwa-rust-coverage.json) is the historical Rust/OpenWA
comparison, not a declaration of Go support. Use `--catalog` for the supported Go API.
## Build and run

Use Go 1.27.1 on Linux. The worker does not link a SQLite implementation;
the binary builds with `CGO_ENABLED=0` and requires no C toolchain. The separate
legacy migration command uses `modernc.org/sqlite`. ADE bundles
are checked in under `assets/`, so a fresh Go build needs neither Node nor pnpm.
Use `GOWORK=off` to build with the dependencies pinned in `go.mod`, regardless
of any local Go workspace.

```sh
cd whatsapp
GOWORK=off make build
./bin/whatsapp --manifest
./bin/whatsapp --catalog
./bin/whatsapp --url ws://127.0.0.1:49134
```

`make build` produces a static release binary with `-trimpath -ldflags="-s -w"`.
To retain native debug symbols, use `make build LDFLAGS=`. Compiler optimizations
and inlining remain enabled in both builds.

The API catalog is embedded as deterministic gzip and decompressed once at
startup. Its readable source remains `catalog.json`; the generated
`catalog.json.gz` is checked in alongside the SDK schema adapters. No build-time
compression tool or extra dependency is required. See
[release size measurements](build-size.md) for sizes and tradeoffs.

The first two inspection commands use neither the network nor session storage.
For frontend changes, install dependencies from the repository root using its
Node and pnpm versions, then rebuild and copy the embedded bundles:

```sh
pnpm install --frozen-lockfile
make -C whatsapp ui
GOWORK=off make -C whatsapp build
```

For isolated development without the configuration worker:

```sh
./bin/whatsapp --local-config --config config.example.yaml --url ws://127.0.0.1:49134
```

Use `--config file.yaml` for a YAML seed. In normal mode, the central entry
`whatsapp` is authoritative; `III_CONFIG_NAME` selects another entry for a
second instance. `III_URL` supplies the engine address. Resource limits and
timeouts apply live; `database`, `data_dir` and `max_events` require a worker restart.
[iii.worker.yaml](../iii.worker.yaml) contains defaults.
Give each process its own `data_dir`; sharing live credential databases between
worker processes is unsupported.

The repository release pipeline builds this worker as `go-binary`, with
`package.json` owning its release version independently of the Go toolchain.
Linux and macOS binaries support amd64 and arm64. Windows is excluded because
session and inbox locking currently require POSIX `flock` and `O_NOFOLLOW`.
See the [Go worker SOP](../../docs/sops/go-binary-worker.md) for the build contract.

For interface discovery, use `--collect-interface --config config.collect.yaml`.
This mode registers the complete typed function interface and triggers against
the engine without fetching central configuration, opening database storage,
reading session/tunnel state, or connecting an account. It rejects business
operation and callback execution. Normal startup still validates its storage
backend; collection mode is only for metadata capture.

### Compatibility with the Rust worker

Use the default `data/whatsapp-go` or another fresh directory. whatsmeow has its
own credential schema, and this worker rejects Rust session metadata. Stop the
old process before reusing its engine identity, then create and explicitly pair
a new Go session. Do not point this executable at the old `data/whatsapp` tree.
The old credentials and archive stay available to the Rust executable; no
automatic credential, archive, event-page or Cloud database migration is provided.
The Go CLI does not implement the Rust `--import-events` command.

An existing central configuration may still select the old directory. Update
that entry to the new directory before starting this worker, or use a separate
`III_CONFIG_NAME` for an isolated instance. Native operation coverage differs
from the Rust version; unsupported calls are omitted from registration.

## Validation

```sh
GOWORK=off make test
GOWORK=off go vet ./...
GOWORK=off go test -race ./...
pnpm --dir ui test
GOWORK=off make ui build
```

Tests cover storage/lifecycle, event bounds and projections, message construction,
addressing, media bounds, strict requests, offline registration and function
schemas. They do not pair an account or send a real WhatsApp message.
Regression tests cover malformed legacy initialization, complete callback
schemas, registration-only CLI startup, rejected retention updates, lock and
database cancellation, and webhook admission. Webhook storage runs under the
request's eight-second deadline with at most eight concurrent deliveries;
excess or timed-out deliveries return HTTP 503 for Meta to redeliver. No detached
archive job continues after a handler returns. Canceled transaction cleanup has
an independent maximum one-second rollback budget. Lost write acknowledgments
can still leave an unknown remote outcome; mutations are never retried.
See the [Go and worker SOP remediation report](go-sop-remediation.md) for
the release contracts and combined validation evidence.
`make catalog` prints the registered schemas. After changing `catalog.json`, run
`make generate` to refresh the SDK schema adapters and compressed catalog, then
rerun tests. Tests compare every embedded contract against the readable source.

[Native functions and events](native-api.md#functions-and-events) defines the native subscription
contract. [Filtered Cloud webhook events](cloud-setup.md#filtered-cloud-webhook-events) defines
the Cloud contract, also exposed as structured schemas through trigger discovery.

### Link the local Go SDK (optional)

To test SDK changes before updating the pinned dependency, link a local checkout
with a temporary Go workspace. From this worker's directory, using the Go version
declared in `go.mod`:

```sh
WHATSAPP_MODULE_DIR="$PWD"
III_GO_SDK_DIR=/path/to/iii-worktree/sdk/packages/go/iii
WHATSAPP_WORKSPACE_DIR="$(mktemp -d)"
(
  cd "$WHATSAPP_WORKSPACE_DIR"
  GOWORK=off go work init "$WHATSAPP_MODULE_DIR" "$III_GO_SDK_DIR"
)
export GOWORK="$WHATSAPP_WORKSPACE_DIR/go.work"
go list -m -json github.com/iii-hq/iii/sdk/packages/go/iii
go test -race ./...
go build -mod=readonly -trimpath -ldflags="-s -w" -o bin/whatsapp .
```

The reported `Dir` must point at the local SDK checkout. This leaves `go.mod`
and `go.sum` unchanged; no absolute local paths are committed. Use `GOWORK=off`
again to build or test the pinned dependency.

### Isolated engine smoke test

The optional engine smoke test starts an isolated engine and worker on loopback
with temporary configuration and storage. Point it at existing local binaries:

```sh
GOWORK=off go build -o /tmp/whatsapp-go-smoke .
III_BINARY=/path/to/iii \
WHATSAPP_BINARY=/tmp/whatsapp-go-smoke \
DATABASE_BINARY=/path/to/database \
HTTP_BINARY=/path/to/http \
GOWORK=off go test -run '^(TestEngineSmoke|TestCloudEnvironmentEngine)$' -count=1 -v .
```

This checks central configuration reloads, runtime schemas, ADE assets, offline
session lifecycle and event delivery across namespaces. `HTTP_BINARY` is optional;
when supplied, it also checks webhook verification, raw-body signatures, account
isolation, duplicate message persistence, filtered Cloud callbacks, callback
namespace/metadata and archive visibility before callback delivery using synthetic
credentials and data.
A synthetic quick-tunnel provider checks the development controls, shared lease
isolation, namespace routing and callback changes through the real engine; it
does not launch cloudflared or expose the test server publicly.
The test does not pair an account or call Meta. Without `III_BINARY`, `WHATSAPP_BINARY` and
`DATABASE_BINARY`, it is skipped.
