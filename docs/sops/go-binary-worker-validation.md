# Go release pipeline validation — 2026-10-07

Validated the current WhatsApp worktree without installing dependencies,
publishing artifacts, contacting Meta, or starting an engine. Go 1.27.1 and the
Python test dependencies were reused from local caches.

## Regression tests

The initial command was:

```sh
PYTHONPATH=/home/anderson/.cache/uv/archive-v0/tdW8ZHo4we1ezq96/lib/python3.13/site-packages \
  /tmp/worker-python-ade-venv/bin/python \
  -m pytest .github/scripts/tests/test_go_deployment.py -q
```

All 17 initial cases failed before implementation: the compiler and catalog
rejected `go-binary`, the Go target mapping was absent, and the Registry artifact
builder rejected Go binaries. Subsequent tests also reproduced the invalid
`0.1.0-go` version-sync input and the compiler digest failing to include the new
target-policy dependency before those issues were fixed.

Final complete script-suite command and output, using the cached Python 3.13
environment with pytest, PyYAML, and jsonschema:

```sh
PYTHONPATH=/home/anderson/.cache/uv/archive-v0/dRQQzoAwW1HAdpwD/lib/python3.13/site-packages \
  PYTHONDONTWRITEBYTECODE=1 \
  /home/anderson/.local/share/uv/python/cpython-3.13.13-linux-x86_64-gnu/bin/python3 \
  -m pytest .github/scripts/tests/ -q
```

```text
308 passed, 3 subtests passed in 3.68s
```

The suite includes all 19 Go cases and regression coverage for existing Rust,
JavaScript, Python, and OCI release paths. All 82 publishable descriptors also
compiled and passed the descriptor JSON Schema; WhatsApp produced four Go build
units and version `0.1.0`.

`validate_worker.py --worker whatsapp --base-ref HEAD --source-changed
'["whatsapp"]'` exited successfully. Forced change discovery placed WhatsApp
in the Go bucket. Actionlint exited successfully for `ci.yml` and
`_deploy-build.yml`; `git diff --check` reported no whitespace errors in the
release scope.

## Actual builds

Each of the following target environments was compiled with the same command
and the cached Go executable:

```sh
CGO_ENABLED=0 GOWORK=off GOTOOLCHAIN=local GOPROXY=off \
  GOOS=<os> GOARCH=<arch> \
  /home/anderson/.cache/whatsapp-go-toolchain/go/bin/go build \
  -mod=readonly -trimpath -ldflags '-s -w -X main.version=0.1.0' \
  -o /tmp/whatsapp-release-s1-<os>-<arch> .
```

| Registry target | GOOS | GOARCH | Result |
|---|---|---|---|
| `x86_64-unknown-linux-gnu` | linux | amd64 | Static, stripped ELF; executable on this host |
| `aarch64-unknown-linux-gnu` | linux | arm64 | Static, stripped ARM64 ELF |
| `x86_64-apple-darwin` | darwin | amd64 | Mach-O x86_64 executable |
| `aarch64-apple-darwin` | darwin | arm64 | Mach-O ARM64 executable |

The Linux amd64 build was also run through `make build`, which reads its
version from `package.json`. Cross-compilation verifies build compatibility;
native macOS and Linux ARM64 behavior was not executed in this environment.
Windows remains excluded because WhatsApp uses POSIX locking APIs.

The real release handlers were exercised for every unit:
`deployment_train.build` → `assemble` → `verify_prepared` →
`deployment_interface.stage`. They produced four archives with checksums and a
verified inventory at:

```text
/tmp/whatsapp-release-s1-validation-ojshqtxi/prepared/prepared-artifacts.json
```

Staging extracted the prepared Linux amd64 executable and retained the exact
hashed configuration seed. Executing the staged command with `--version`
returned:

```text
whatsapp 0.1.0
```

The staged capture command was:

```sh
/tmp/whatsapp-release-s1-validation-ojshqtxi/capture/runtime/whatsapp \
  --config /home/anderson/project/iii/workers/.worktrees/whatsapp-go/whatsapp/config.collect.yaml \
  --local-config --collect-interface
```

This report verifies build and staging behavior from the worktree; no release
was published.

## Combined verification after runtime fixes

The final Go source was rebuilt through all four release handlers, assembled,
inventory-verified, and staged at `/tmp/whatsapp-go-sop-final-yjgm1wny/`.
Actual registration-only capture from its prepared Linux amd64 executable
discovered all **239 functions and 2 triggers**, with request/payload schemas
for every entry and no schema warnings. It required neither database nor
configuration sidecars and created no worker storage.

This integration check exposed an asynchronous-registration race in the shared
collector. Positive waits now require three unchanged nonempty function ID
snapshots; unstable/empty captures fail without writing metadata. Trigger-only
workers preserve their deadline timing and require stable registrations owned
by the selected provider. The collector regressions bring the complete script
suite to **322 tests and 3 subtests passed**, with no skips.

The [worker remediation report](../../whatsapp/docs/go-sop-remediation.md)
records the final Go/race/UI checks and isolated engine/database/HTTP tests.
