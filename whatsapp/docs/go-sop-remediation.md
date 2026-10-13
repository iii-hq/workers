# Go and worker SOP remediation — 2026-10-07

This change implements the seven findings from the October 7 audit of the
WhatsApp Go worktree. Existing Cloud API, UI, and database-storage work remains
part of the worktree; this report describes the audit fixes.

## Resolved findings

| Finding | Result | Main implementation |
|---|---|---|
| S1: missing Go release recipe | WhatsApp has a private catalog entry, a `go-binary` recipe, explicit version ownership, static stripped artifacts, checksums, and Registry projection. | `.deploy/workers.yaml`, `.github/scripts/deployment_*.py`, CI workflows, `whatsapp/package.json`, `iii.worker.yaml`, `Makefile` |
| S2: capture required a database | `--collect-interface` registers the production interface through an ephemeral manager without configuration/database initialization, session files, or tunnel state. Registered business operations reject execution. | `main.go`, `interface.go`, capture guards in callback registrations |
| S3: untyped internal callbacks | All seven internal configuration, webhook, Flow, and tunnel callbacks use typed SDK adapters. HTTP callbacks preserve their existing HTTP 400 responses for malformed fields. | `callback_schemas.go`, `callback_handlers.go`, `config.go`, webhook/Flow/tunnel registrations |
| S4: malformed legacy reads authorized writes | Only an explicitly present null `value` or exact `NOT_FOUND` permits seeding. Other malformed envelopes abort. Legacy fallback emits one secret-safe warning per entry/process. | `config.go`, configuration initialization regressions |
| S5: public operations hidden from traces | Catalog operations publish visible spans. Internal plumbing retains hidden metadata; SDK payload capture remains disabled. | `registry.go`, `main.go`, registration-boundary regression |
| G1: rejected retention reload deleted earlier messages | Reload validates and locks every inbox before publishing limits/configuration. Reload itself does not purge rows; a subsequent successful archive transaction enforces the accepted limit. | `sessions.go`, `inbox.go`, injected-storage-failure regression |
| G2: cancellation lost in storage and detached webhook work | Request contexts reach manager/inbox locks, database RPCs, SQL transactions, lifecycle operations, event polling, media bounds, and tunnel/configuration callbacks. Webhooks process synchronously under an eight-second deadline with eight admission slots. | Manager/inbox context methods, `internal/databasestore`, Cloud/native request handlers, `events.go`, `native_media.go`, `cloud_webhook.go`, `cloud_tunnel.go` |

The storage adapter never retries a mutation whose acknowledgment is lost.
Cancellation cleanup uses an independent rollback budget of at most one second.
A timeout may still leave an unknown remote result; it does not prove that a
send or database commit failed.

Actual artifact capture also exposed a registration race: the collector took a
snapshot after the first function batch, yielding only 86 of 239 functions.
Positive waits now require three unchanged nonempty target registration samples
before collecting a function interface. No-wait behavior remains unchanged;
empty or unstable interfaces fail instead of writing partial metadata.
Trigger-only workers retain the previous deadline timing and require stable,
attributable trigger registrations. Late functions reset readiness. This path
has dedicated regression coverage.
The native `whatsapp::events` trigger now advertises filter and event-envelope
schemas, preserving omitted/null filters and generic event data.

## Release and interface contracts

- Version metadata: `whatsapp/package.json`, currently `0.1.0`. It is a metadata
  file and introduces no Node runtime or package-install requirement.
- Toolchain: Go 1.27.1, matching `go.mod`; pinned SDK dependencies remain unchanged.
- Release: `CGO_ENABLED=0 GOWORK=off GOTOOLCHAIN=local go build -mod=readonly
  -trimpath -ldflags '-s -w -X main.version=0.1.0'`.
- Targets: Linux amd64/arm64 and macOS amd64/arm64. Windows is excluded because
  session/inbox locking uses POSIX `flock` and `O_NOFOLLOW`.
- Capture: `whatsapp --config config.collect.yaml --local-config --collect-interface`
  connects only to the selected engine and waits for shutdown after registration.
  This mode also advertises the central configuration callback.
- Runtime dependencies retain the repository's `latest` selector policy. Normal
  worker startup still validates the configured database worker.

See [Go binary worker SOP](../../docs/sops/go-binary-worker.md) and
[release pipeline validation](../../docs/sops/go-binary-worker-validation.md).

## Verification

Executed against the final Go source with the cached Go 1.27.1 toolchain and
`GOWORK=off`:

| Check | Result |
|---|---|
| `CGO_ENABLED=0 go test -mod=readonly ./... -count=1` | Passed |
| `CGO_ENABLED=0 go vet -mod=readonly ./...` | Passed |
| `go test -race ./... -count=1` | Passed |
| Complete deployment script suite | 322 tests and 3 subtests passed, with no skips |
| Recursive `gofmt -l` and `git diff --check` | Clean |
| Worker manifest validator and actionlint for CI/build workflows | Passed |
| Deployment descriptor compilation and JSON Schema | All 82 publishable descriptors passed |
| Four real `deployment_train build` units, assembly, inventory verification, and interface staging | Passed; staged executable reports `whatsapp 0.1.0` |
| Actual engine capture from the prepared Linux amd64 artifact, without database/configuration sidecars | 239 functions and 2 triggers; both schemas present for every entry; no schema warnings or storage creation |
| Existing UI tests | 180 passed across 10 files |
| UI type check, strict build, and embedded asset comparison | Passed; zero lint errors/warnings; embedded JS/CSS identical to build output |
| `TestEngineSmoke` with local engine, database, and HTTP binaries | Passed, including central reload, all public schemas, ADE assets, offline lifecycle, webhook signatures/deduplication, Cloud filters, namespace/metadata, and synthetic tunnel controls |
| `TestDatabaseWorkerPersistence` and `TestDatabaseWorkerLegacyMigration` | Passed with synthetic native keys and archives |

The artifact capture compares the complete discovered ID set with the binary's
catalog plus eight internal functions, rather than relying on a nonempty result.
Its evidence and all four archives are retained temporarily at
`/tmp/whatsapp-go-sop-final-yjgm1wny/`. The initial polling-cancellation regression
also failed as expected against an overlay restoring the old unconditional lock,
then passed with the fix.

Independent reviews checked release integration, runtime cancellation, native
trigger schemas, and the collector changes. Additional findings from those
reviews were fixed and covered by regressions; the final rechecks reported no
remaining concrete issues in those changes.

These are local worktree validations, not published release provenance. Linux
amd64 integration was executed; Linux ARM64 and macOS were cross-compiled only.
Other opt-in engine tests are skipped by the default unit suite unless their
required fixture environment is supplied. Offline tests do not establish live
WhatsApp account interoperability.

All account data used by integration tests is synthetic; temporary engines bind
only to loopback. No production configuration, native account pairing, real
sends, or Meta API calls were performed. No dependencies were installed and no
live engine was restarted.
