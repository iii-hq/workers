# Release binary size

The default `make build` now strips native debug information and embeds a
compressed API catalog. The worker keeps all native, Cloud API, and ADE features,
compiler optimizations, and inlining. It remains one static Go executable.
Native keys and archives now use the existing database worker, so the runtime
binary also excludes the SQLite implementation. See [storage and migration](database-storage.md).

## Measurements

Measured on October 5, 2026, using Go 1.27.1, linux/amd64, `CGO_ENABLED=0`,
`GOWORK=off`, and the current `feat/whatsapp-go` worktree, including its existing
Cloud API and UI changes. Sizes are executable file bytes, excluding session
storage and configuration. MB uses 1,000,000 bytes; MiB uses 1,048,576 bytes.

| Build | Bytes | MB | MiB |
| --- | ---: | ---: | ---: |
| Original build with native debug information | 32,466,301 | 32.47 | 30.96 |
| Original source, `-ldflags="-s -w"` | 22,659,232 | 22.66 | 21.61 |
| Compressed catalog, before database-worker storage | 22,184,096 | 22.18 | 21.16 |
| Default build, database-worker storage | 18,329,760 | 18.33 | 17.48 |
| No-inlining experiment, original catalog, rejected | 21,368,992 | 21.37 | 20.38 |

The current default build saves 14,136,541 bytes (43.54%) relative to the original debug build.
Moving SQL execution to the database worker saves 3,854,336 bytes (17.37%)
relative to the already stripped release with compressed catalog. The separate
`whatsapp-migrate` utility remains SQLite-based and is not needed during normal operation.
The SQL storage remains part of the deployment in the existing database worker.
Catalog compression saves another 475,136 bytes (2.10%) relative to the already
stripped release. The catalog itself changes from 504,927 embedded bytes to
31,849 compressed bytes. Native debug information is removed according to the
[Go linker flags](https://pkg.go.dev/cmd/link); `LDFLAGS=` retains it when needed.

## Build commands

From this module, with Go 1.27.1 on `PATH`:

```sh
# Default static release build.
GOWORK=off make build

# Separate artifact without replacing the executable used by a live worker.
GOWORK=off make build BIN=bin/whatsapp-release

# Keep debug information for native debugger use.
GOWORK=off make build LDFLAGS= BIN=bin/whatsapp-debug

./bin/whatsapp-release --version
./bin/whatsapp-release --manifest
./bin/whatsapp-release --catalog
```

`catalog.json` remains the readable contract source. After editing it, run
`make generate` to update both `catalog_generated.go` and `catalog.json.gz`.
The gzip header omits timestamps and names; repeated generation with the same
toolchain produces identical bytes. Both generated files must be included with
source changes so a fresh build needs no extra tooling.

The compressed catalog is decompressed once during initialization using the Go
standard library, and the temporary decoded byte buffer can then be collected.
Function registration, typed schemas, request validation, and catalog output
continue using the same parsed contracts. Catalog compression adds no dependency
or external compression program. A test compares every embedded contract to its readable source
and fails if the generated artifact is stale.

## Rejected size tradeoff

The experiment `-gcflags=all=-l -ldflags="-s -w"` disables inlining, as documented
by the [Go compiler](https://pkg.go.dev/cmd/compile). On the original source it
saved only 1,290,240 bytes (5.69%) relative to stripping alone. Six serial samples
per workload on the same host showed:

| Workload | Normal inlining, median ms/op | Inlining disabled, median ms/op |
| --- | ---: | ---: |
| Decode a document request containing a 1 MiB file | 2.901 | 5.238 |
| Replay one native receipt in a 10,000-message chat | 1.984 | 44.936 |

Receipt allocations also increased from about 788 bytes to 19.1 MB per operation.
These controlled workloads are not production throughput measurements, but the
large regression makes this unsuitable as the default size optimization.
The no-inlining experiment preceded catalog compression; its size is not the
size of a combined no-inlining/compressed-catalog build.

Raw measurements are retained in
[normal-inlining samples](build-size/2026-10-05/inline-benchmark.txt) and
[no-inlining samples](build-size/2026-10-05/noinline-benchmark.txt). Reproduce them
serially, with the same toolchain and source:

```sh
GOWORK=off CGO_ENABLED=0 go test -run '^$' \
  -bench 'Benchmark(DecodeCloudDocumentRequest|InboxRecordReceipt)/(fileMiB=1|messages=10000)$' \
  -benchmem -benchtime=300ms -count=6 -cpu=1 .

GOWORK=off CGO_ENABLED=0 go test -gcflags=all=-l -run '^$' \
  -bench 'Benchmark(DecodeCloudDocumentRequest|InboxRecordReceipt)/(fileMiB=1|messages=10000)$' \
  -benchmem -benchtime=300ms -count=6 -cpu=1 .
```

An alternating 20-sample CLI startup check (`--version`) produced overlapping
ranges: 9.0–40.3 ms before catalog compression and 9.9–40.3 ms after. No startup
speed improvement is claimed from this noisy development-host check.

## Validation and scope

The following checks passed:

- Static stripped release build using the changed Makefile.
- `go test ./...` and `go vet ./...`.
- Exact semantic equality of `--manifest` and the complete `--catalog` output,
  containing 231 registered functions, before and after the optimization.
- Repeated generation with unchanged hashes for both generated artifacts.
- A deliberately stale compressed catalog supplied through a temporary Go
  overlay was correctly rejected by the new contract-parity test.
- Existing UI embedding, native transport, Cloud API payload, webhook, and
  no-retry tests included in the Go suite.

These checks use synthetic fixtures and inspection commands. No real WhatsApp
messages were sent. The optimized artifact was built as `bin/whatsapp-release`;
the running Live Engine worker was not replaced or restarted.

This remains a local build recipe. Adding Go artifacts to the repository's
package-release pipeline is a separate task. The runtime release includes Go's
runtime metadata, WhatsApp/protobuf implementations, and all current worker
functions. SQLite remains in the separate migration utility and the database
worker, rather than the WhatsApp runtime executable.

An [explicit-schema registration spike](build-size/2026-10-06-schema-spike/README.md)
measured a further reduction to 17,379,488 bytes (5.18%) without removing worker
features. That prototype has not been applied to the SDK or the worker.
