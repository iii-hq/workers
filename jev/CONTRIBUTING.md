# Contributing to JEV

Use the [README](README.md) to install and call the worker, and the
[API reference](reference.md) for request, response and operational contracts.
Run the commands below from the repository root. JEV is an isolated Cargo
workspace; its UI belongs to the root pnpm workspace.

## Build from source

Install Rust, Node.js and the pnpm version declared in the
[root package manifest](https://github.com/iii-hq/workers/blob/main/package.json).
Build the UI before compiling a worker with the default `console-ui` feature:

```bash
pnpm --filter '@iii-workers/jev-ui...' install --frozen-lockfile
pnpm --dir jev/ui build
cargo build --manifest-path jev/Cargo.toml --locked --release
```

The binary is `jev/target/release/jev` unless `CARGO_TARGET_DIR` overrides the
output directory. To build without the Console form or its Node/pnpm dependency:

```bash
cargo build --manifest-path jev/Cargo.toml --locked --release --no-default-features
```

For a standalone development process, use engine **`iii/v0.24.0-rc.2`** with
`configuration::ensure` available. This is the verified release pinned by CI.
If making provider calls, supply a key securely through configuration or the
JEV process environment:

```bash
# TYPESAFE_API_KEY must be present in this process environment.
cargo run --manifest-path jev/Cargo.toml --locked -- --url ws://127.0.0.1:49134
```

`III_URL` is the environment equivalent of `--url`. Append `--config <path>` to
seed a missing configuration entry with YAML or JSON; stored values take
precedence. `III_CONFIG_NAME` selects the entry ID. The worker can boot without
provider credentials; evaluation and model listing then return `missing_key`
without HTTP. See [configuration](reference.md#configuration) for reload rules.

## UI development

The UI registers the `jev` configuration form. The standard Console builder
emits `ui/dist/page.js` and `ui/dist/styles.css` for Rust embedding.

```bash
pnpm --dir jev/ui test
pnpm --dir jev/ui build
```

`test` runs Vitest; `build` runs TypeScript checking and the shared builder's
strict Console UI linting. The form tests cover credential clearing, numeric
limits, preserving unknown values and configuration-service errors.

For live UI editing, run these in separate terminals from the repository root:

```bash
pnpm --dir jev/ui watch
III_JEV_UI_WATCH=jev/ui/dist cargo run --manifest-path jev/Cargo.toml --locked -- --url ws://127.0.0.1:49134
```

The explicit watch path resolves from the repository root. `1` or `true` uses
`ui/dist` relative to the worker process's current directory, so those values
apply when starting from `jev/`. Without the watch environment, rebuild the binary
to embed changed UI assets. [build.rs](build.rs) builds missing or stale assets
automatically. `PNPM=<path>` selects the pnpm executable. `SKIP_UI_BUILD=1`
reuses existing assets, even if stale, and fails if either is missing; use it
only after building the current UI. `--no-default-features` skips UI building
and registration entirely.

## Rust checks

After building the UI, run the shared contract suite and both worker variants:

```bash
unset TYPESAFE_API_KEY
cargo test --manifest-path crates/jev-contract/Cargo.toml --locked
cargo fmt --manifest-path jev/Cargo.toml --all -- --check
cargo clippy --manifest-path jev/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path jev/Cargo.toml --locked --all-features
cargo clippy --manifest-path jev/Cargo.toml --locked --all-targets --no-default-features -- -D warnings
cargo test --manifest-path jev/Cargo.toml --locked --no-default-features
```

The default suites use local HTTP/WebSocket fixtures and need no provider key
or running engine. The two real-engine cases are ignored unless explicitly
selected. Coverage includes strict schemas, mixed answers, configuration
snapshots, request limits, retry deadlines, bounded/redacted provider errors,
usage accounting, cancellation ownership, and binary boot/registration.
The default-feature run also covers embedded Console assets; the second run
checks the worker without UI support.

For a focused transport or registration change:

```bash
cargo test --manifest-path jev/Cargo.toml --locked --test client
cargo test --manifest-path jev/Cargo.toml --locked --test register
cargo test --manifest-path jev/Cargo.toml --locked --test boot
```

## Real-engine integration with local mocked HTTP

The dedicated [JEV E2E workflow](https://github.com/iii-hq/workers/blob/main/.github/workflows/jev-e2e.yml)
runs on pull requests only. It pins **`iii/v0.24.0-rc.2`**, runs UI Vitest and both
Rust feature variants, and selects two standalone bus cases from
[tests/engine.rs](tests/engine.rs):

| Test | Coverage |
| --- | --- |
| `independent_consumer_evaluates_mixed_primitives_and_lists_models` | Mixed Noul/Choice/Score answers, nullable usage, model listing, invalid requests, provider errors, retry hints and credential redaction. |
| `cancellation_is_scoped_to_the_persistent_engine_caller` | An independent caller cannot cancel another caller's evaluation; its persistent owner can interrupt it and receives typed cancellation stats. |

Each case starts a fresh engine and registers the production JEV handlers through
its Rust library. Provider HTTP is mocked on loopback. These tests exercise real
bus routing and engine-supplied caller identity. The separate `boot` suite tests
the worker executable against a WebSocket fixture. No inference backend or
provider credential is needed.

After building the JEV UI, set `III_ENGINE_BIN` to an absolute path to the pinned
engine and run the same entry point as CI:

```bash
export III_ENGINE_BIN=/absolute/path/to/iii
bash .github/scripts/jev-e2e.sh
```

The runner clears `TYPESAFE_API_KEY`, selects the two ignored tests by exact name
and runs them serially. Empty selections, renamed tests and failures fail the
run. It prints the report directory and retains Cargo output and each engine's
stdout/stderr, including on failures. Set `JEV_E2E_REPORT_DIR` to choose that
directory; otherwise a new temporary report directory is created. Each fixture
kills and reaps its engine before removing the engine's separate scratch data.
CI uploads the reports even when a preceding step fails.

The tests build only JEV and its dependencies. `CARGO_TARGET_DIR` can reuse a
local build cache. The runner uses the existing release profile; it does not
change release optimization settings. Normal `cargo test` skips these ignored
cases. Keep exact test selection when adding scenarios so future live-provider
tests cannot run inadvertently.

## Worker conventions

The public runtime functions remain `jev::evaluate`, `jev::models::list` and
`jev::cancel`. Their types live in
[jev-contract](https://github.com/iii-hq/workers/tree/main/crates/jev-contract),
and [src/register.rs](src/register.rs) publishes the bus schemas. Configuration
reload and Console asset handlers are infrastructure, outside that public API.
The [worker skill](skills/SKILL.md) describes agent-facing invocation guidance.

Follow the repository's [worker README guide](https://github.com/iii-hq/workers/blob/main/worker-readme.md),
[new-worker SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/new-worker.md),
[Rust binary SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/binary-worker.md),
[configuration SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/configuration.md)
and [injectable Console UI SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/injectable-console-ui.md).
Keep consumer examples in English, deep API details in [reference.md](reference.md),
and source recipes here. Repository links outside `jev/` must use absolute
GitHub URLs so the published README works outside GitHub.
