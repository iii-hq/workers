# Contributing to judge-openai

Use the [README](README.md) to install and call the worker, and the hub's
[API reference](../judge/reference.md) for request, response and operational contracts.
Run the commands below from the repository root. judge-openai is an isolated Cargo
workspace; its UI belongs to the root pnpm workspace.

## Build from source

Install Rust, Node.js and the pnpm version declared in the
[root package manifest](https://github.com/iii-hq/workers/blob/main/package.json).
Build the UI before compiling a worker with the default `console-ui` feature:

```bash
pnpm --filter '@iii-workers/judge-openai-ui...' install --frozen-lockfile
pnpm --dir judge-openai/ui build
cargo build --manifest-path judge-openai/Cargo.toml --locked --release
```

The binary is `judge-openai/target/release/judge-openai` unless `CARGO_TARGET_DIR` overrides the
output directory. To build without the Console form or its Node/pnpm dependency:

```bash
cargo build --manifest-path judge-openai/Cargo.toml --locked --release --no-default-features
```

For a standalone development process, use engine **`iii/v0.24.0-rc.2`** with
`configuration::ensure` available. This is the verified release pinned by CI.
If making provider calls, supply a key through configuration (preferably
`secret://OPENAI_API_KEY`) or the worker's environment:

```bash
# OPENAI_API_KEY must be present in this process environment.
cargo run --manifest-path judge-openai/Cargo.toml --locked -- --url ws://127.0.0.1:49134
```

`III_URL` is the environment equivalent of `--url`; `III_CONFIG_NAME` selects
the configuration entry ID. The worker can boot without provider credentials;
evaluation and model listing then return `missing_key` without HTTP. See
[configuration](../judge/reference.md#configuration) for reload rules.

## UI development

The UI registers the `judge-openai` configuration form. The standard Console builder
emits `ui/dist/page.js` and `ui/dist/styles.css` for Rust embedding.

```bash
pnpm --dir judge-openai/ui test
pnpm --dir judge-openai/ui build
```

`test` runs Vitest; `build` runs TypeScript checking and the shared builder's
strict Console UI linting.

For live UI editing, run these in separate terminals from the repository root:

```bash
pnpm --dir judge-openai/ui watch
III_JUDGE_OPENAI_UI_WATCH=judge-openai/ui/dist cargo run --manifest-path judge-openai/Cargo.toml --locked -- --url ws://127.0.0.1:49134
```

The explicit watch path resolves from the repository root. `1` or `true` uses
`ui/dist` relative to the worker process's current directory. Without the watch
environment, rebuild the binary to embed changed UI assets. [build.rs](build.rs)
builds missing or stale assets automatically. `PNPM=<path>` selects the pnpm
executable. `SKIP_UI_BUILD=1` reuses existing assets, even if stale, and fails if
either is missing. `--no-default-features` skips UI building and registration entirely.

## Rust checks

After building the UI, run the shared contract and provider suites and both
worker variants:

```bash
unset OPENAI_API_KEY
cargo test --manifest-path crates/judge-contract/Cargo.toml --locked
cargo test --manifest-path crates/judge-provider/Cargo.toml --locked
cargo fmt --manifest-path judge-openai/Cargo.toml --all -- --check
cargo clippy --manifest-path judge-openai/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path judge-openai/Cargo.toml --locked --all-features
cargo clippy --manifest-path judge-openai/Cargo.toml --locked --all-targets --no-default-features -- -D warnings
cargo test --manifest-path judge-openai/Cargo.toml --locked --no-default-features
```

The default suites use local HTTP/WebSocket fixtures and need no provider key or
running engine; in-process clients always use a local test key and never read the
environment. The two real-engine cases are ignored unless explicitly selected.

| Suite | Coverage |
| --- | --- |
| `src/decisions.rs` | Request translation, decoding by name, local one-option choices, refusals, model cards. |
| `tests/client.rs` | Keys, limits, deadlines, retries and Retry-After, billing 429s, auth error stripping, shared permits, cancellation, usage accounting, model filtering. |
| `tests/consumers.rs` | The exact Decisions bodies for iii-directory, harness and `browser::run` request shapes, and their reply rules. |
| `tests/boot.rs`, `tests/register.rs`, `tests/config.rs`, `tests/configuration.rs`, `tests/secrets.rs` | Binary boot and registration, strict schemas, configuration lifecycle, `secret://` resolution and rotation. |

HTTP retries, Retry-After and billing 429 handling, bounded/redacted provider
errors, the cancellation registry and `secret://` resolution live in the shared
[crates/judge-provider](https://github.com/iii-hq/workers/tree/main/crates/judge-provider).
Make transport changes there, run its suite, then rerun this worker's suites. The
Decisions wire format lives in [src/decisions.rs](src/decisions.rs).

The bus test fixtures in `judge-typesafe/tests/support/` are compiled through
`#[path]` by the other judge workers and crates/judge-provider; a change there
must keep all of those suites passing.

## Real-engine integration with local mocked HTTP

The dedicated [Judge E2E workflow](https://github.com/iii-hq/workers/blob/main/.github/workflows/judge-e2e.yml)
runs on pull requests only. It pins **`iii/v0.24.0-rc.2`** and selects two
standalone bus cases from [tests/engine.rs](tests/engine.rs):

| Test | Coverage |
| --- | --- |
| `independent_consumer_evaluates_mixed_decisions_and_lists_models` | Mixed Noul/Choice/Score answers with a local one-option choice, the exact Decisions body, the filtered model catalog, invalid requests, 429 retries with `retry-after-ms` and credential redaction. |
| `cancellation_is_scoped_to_the_persistent_engine_caller` | An independent caller cannot cancel another caller's evaluation; its persistent owner can interrupt it and receives typed cancellation stats. |

Each case starts a fresh engine and registers the production handlers through
the Rust library; Decisions HTTP is mocked on loopback. Set `III_ENGINE_BIN` to an
absolute path to the pinned engine and run the same entry point as CI:

```bash
export III_ENGINE_BIN=/absolute/path/to/iii
bash .github/scripts/judge-e2e.sh
```

The runner clears provider keys and selects every case by exact name. Keep exact
test selection when adding scenarios so live-provider calls cannot run
inadvertently.

## Worker conventions

The public runtime functions remain `judge-openai::evaluate`, `judge-openai::models::list` and
`judge-openai::cancel`. Their types live in
[judge-contract](https://github.com/iii-hq/workers/tree/main/crates/judge-contract),
and [src/register.rs](src/register.rs) publishes the bus schemas. Every
registration carries `metadata.internal: true`: callers reach the provider
through the `judge` hub, and configuration reload, secret rotation and Console
asset handlers are infrastructure.

Follow the repository's [worker README guide](https://github.com/iii-hq/workers/blob/main/worker-readme.md),
[new-worker SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/new-worker.md),
[Rust binary SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/binary-worker.md),
[configuration SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/configuration.md)
and [injectable Console UI SOP](https://github.com/iii-hq/workers/blob/main/docs/sops/injectable-console-ui.md).
Keep consumer examples in English, deep API details in [reference.md](../judge/reference.md),
and source recipes here. Repository links outside `judge-openai/` must use absolute
GitHub URLs so the published README works outside GitHub.
