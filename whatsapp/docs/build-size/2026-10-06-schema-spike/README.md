# Explicit-schema registration spike

The probe reduced the static WhatsApp executable from **18,329,760 to
17,379,488 bytes**, saving **950,272 bytes (5.18%)**. All 231 operation
contracts remained identical. This is a throwaway prototype, not an applied
SDK or worker change. The running Live Engine worker was not restarted.

Measured October 6, 2026, with Go 1.27.1, linux/amd64, `CGO_ENABLED=0`,
`GOWORK=off`, `-trimpath`, and `-ldflags='-s -w'`. The worker source was the
existing dirty `feat/whatsapp-go` worktree at `d6788be0f`, including the
database-worker storage changes. The SDK was the exact pinned module
`v0.24.6-rc.1.0.20261002194513-a075c6e40389`, copied from the module cache.

## What the probe changed

The prototype adds `RequestFormat` and `ResponseFormat`, both
`json.RawMessage`, to `iii.RegisterFunctionOptions`. Direct registration
advertises these schemas without inferring them from Go types. Empty values
preserve the previous registration shape. Nonempty schemas must be valid JSON;
the SDK copies their bytes and replays them after reconnecting. Invalid input
leaves an existing handler unchanged and errors omit the schema contents.

The prototype also accepts explicit schemas in `RegisterFunctionTyped`;
unspecified schemas retain inference and the handler still receives its typed
request. Schemas remain discovery metadata, not automatic request validation.

The worker registers the existing catalog schemas directly, removing 231
generated operation tags and their request/response adapters. It keeps the
existing JSON Schema request validator, timeouts, metadata, flat responses,
and native and Cloud API handlers. The UI endpoint also registers its existing
schemas directly, including the original `$schema` and `$id` fields.

## Size results

| Variant | Executable bytes | MB |
| --- | ---: | ---: |
| Baseline, all existing features | 18,329,760 | 18.33 |
| Direct catalog registration, typed UI retained | 17,432,736 | 17.43 |
| Direct catalog and UI registration | 17,379,488 | 17.38 |

The second variant saves 897,024 bytes. Direct UI registration saves another
53,248 bytes. Most of the total saving comes from eliminating generated code
and its type and function metadata, rather than removing dependencies wholesale.
The final executable still contains some schema-library initialization code.

The final executable compresses to **6,298,301 bytes with gzip** (`-n -9`) or
**5,206,584 bytes with zstd** (`-19`). Both decompressed outputs matched the
executable byte for byte. These are compressed-file sizes, not installed
executable sizes; the corresponding baseline files were 6,460,104 and
5,320,746 bytes.

## Registration benchmark

Six serial samples per variant on an Intel Core i9-14900K, one Go CPU, with
300 ms per sample. This measures installation of the 231 function registrations
into a disconnected SDK client. It excludes network traffic, request-schema
compilation, and message processing; it is not whole-worker startup latency.

| Measurement | Baseline | Direct registration |
| --- | ---: | ---: |
| Median time per installation | 7.24 ms | 4.03 ms |
| Allocated bytes per installation, approximately | 4.14 MB | 1.91 MB |
| Allocations per installation | 26,156 | 15,755 |

See [baseline samples](baseline-install-benchmark.txt) and
[direct-registration samples](explicit-install-benchmark.txt).

## Validation

Passed:

- SDK tests, race detection, and `go vet`, including explicit-schema copying,
  reconnect replay, nullable and boolean schemas, optional schemas, invalid
  registration rejection, and typed-handler compatibility.
- Both worker variants' unit suites; the final variant also passed the complete
  default `go test -race ./...` suite and `go vet ./...`.
- Local WebSocket assertions for all 231 catalog contracts and their metadata,
  the complete UI schemas, a flat UI object response, and a flat session-list
  array response. The UI comparison caught a missing `$id` in the initial
  prototype; the final variant preserves it.
- Byte-identical `--catalog` and `--manifest` output before and after.
- Real isolated engine/database tests: `TestEngineSmoke`,
  `TestDatabaseWorkerPersistence`, and `TestDatabaseWorkerLegacyMigration`.
  The final binary was checked again with `TestEngineSmoke` after preserving
  the UI schema IDs. See [integration results](engine-integration.txt) and
  [final binary smoke test](final-engine-smoke.txt).
- Original SDK-cache and worker registration source fingerprints remained
  unchanged. Product changes were supplied through temporary Go overlays and
  a separate modfile, not written into those sources.

The full suite with integration environment variables enabled failed in
`TestCloudEnvironmentEngine/seed`, `/existing`, and `/local`. Its fixture starts
the engine and WhatsApp but does not start the database worker now required at
startup. `/seed` failed with the same suppressed database error against the
baseline executable, confirming this is independent of the schema probe.
See the [baseline failure summary](baseline-env-failure.txt).
The normal suite skips this opt-in integration test. Its fixture needs a
database worker before that integration gate can pass.

Native and Cloud API payload, webhook, and no-retry unit tests were included.
No real WhatsApp messages were sent and no Meta configuration was changed.
The optional HTTP-listener smoke test was not enabled.

## Reproduce while the temporary probe exists

Prototype files and executables are isolated at
`/tmp/whatsapp-schema-spike-20261006.0JAsEk`. `sdk/` contains the experimental
SDK; `registry.go` and `ui.go` are worker overlays. `adapters.json` and
`explicit.json` select the two variants. The repository's `go.mod` and `go.sum`
remain unchanged.

From the WhatsApp module, with Go 1.27.1 on `PATH`:

```sh
WHATSAPP_SCHEMA_SPIKE=/tmp/whatsapp-schema-spike-20261006.0JAsEk

CGO_ENABLED=0 GOWORK=off go build \
  -modfile "$WHATSAPP_SCHEMA_SPIKE/worker.go.mod" \
  -overlay "$WHATSAPP_SCHEMA_SPIKE/explicit.json" \
  -trimpath -ldflags='-s -w' \
  -o "$WHATSAPP_SCHEMA_SPIKE/explicit" .

GOWORK=off go test -race \
  -modfile "$WHATSAPP_SCHEMA_SPIKE/worker.go.mod" \
  -overlay "$WHATSAPP_SCHEMA_SPIKE/explicit.json" ./...

GOWORK=off go vet \
  -modfile "$WHATSAPP_SCHEMA_SPIKE/worker.go.mod" \
  -overlay "$WHATSAPP_SCHEMA_SPIKE/explicit.json" ./...
```

## Recommendation

Adopt explicit-schema registration in a separate SDK change, then simplify the
worker against that released capability. The saving is measurable while
preserving all current worker features. Keeping the prototype as production
code requires that follow-up implementation and SDK API review. Correct the
existing environment-integration fixture as part of the database-worker rollout.
