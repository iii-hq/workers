# Binary worker how-to

Cross-cutting steps → [`new-worker.md`](new-worker.md). Shipping → [`release.md`](release.md).

This document is the **single source of truth** for building a Rust **binary**
iii worker: a single cross-compiled CLI that connects to the iii engine over
WebSocket, registers functions and triggers, and runs until SIGTERM. Everything
you need to scaffold one is **here**—layouts, contracts, and copy-paste snippets.

For the cross-cutting checklist (folder names, registry, CI, release flow)
see [`new-worker.md`](new-worker.md). This document covers the **inside** of
one such worker: the manifest, config, layout, function registration, trigger
registration, end-to-end tests, and README contract.

If your worker is not a Rust binary (`deploy: image`, or any Node/Python
worker), stop here and read [`new-worker.md`](new-worker.md) instead — the
inside of those workers looks different.

## 1. When to use this doc

Use this scaffold exactly when **all** of the following are true:

- `iii.worker.yaml.deploy: binary`
- `iii.worker.yaml.language: rust`
- The worker is a long-running daemon (it stays connected to iii and serves
  invocations until killed).

Resulting shape:

- One Cargo crate with one `[[bin]]`.
- Connects to `ws://127.0.0.1:49134` (or `--url`) on startup via
  `iii_sdk::register_worker`.
- Registers N functions and M triggers, then `tokio::signal::ctrl_c().await`.
- Exits cleanly via `iii.shutdown_async().await` on SIGINT/SIGTERM.
- Supports `--manifest` for the registry publish pipeline (prints JSON, exits 0).

## 2. Project layout

A binary worker lives in a single folder at the repo root. **`<worker>`** is the
folder name (same value as `iii.worker.yaml` `name`, `[package].name`,
`[[bin]].name`, `iii.worker.yaml` `bin`, and `WorkerMetadata.name`). Do **not**
prefix it with `iii-`.

### Required files

- `iii.worker.yaml` — registry manifest. See section 3.
- `Cargo.toml` — crate manifest with one `[[bin]]`. See section 4.
- `build.rs` — exposes the build-time `TARGET` triple to the binary. See section 4.
- `config.yaml` — **Path A only** (static config): operator-facing runtime
  config, committed, loaded by `--config ./config.yaml`. **Path B** (configuration
  worker): omit — see section 5 and [`configuration.md`](configuration.md).
- `src/main.rs` — entry point. See section 6.
- `src/config.rs` — `Config` struct; Path A adds `load_config`. Path B adds
  `json_schema()` / `from_json` / `boot_signature()`. See section 5.
- `src/manifest.rs` — `build_manifest()` for the `--manifest` subcommand. See section 5.
- `src/state.rs` — thin `state::get` / `state::set` wrappers around `iii.trigger()`. Optional but recommended. See section 7.
- `src/functions/mod.rs` — `register_all(...)` plus one `register_<verb>` per function. See section 7.
- `src/functions/<verb>.rs` — input/output types (`JsonSchema` + serde) and helpers for that function. See section 7.
- `tests/` — non-empty. See section 9 (choose a test pattern).
- `README.md` — required; structure in section 10.

### Optional: library crate (`src/lib.rs`)

Add a **`[lib]`** target when:

- integration or BDD tests need to call `register_all`, handlers, or shared
  helpers **in process**, or
- you want the binary to stay a thin `main` and keep all registration logic
  in a library.

Rules:

- `[package] name` stays **`<worker>`** (hyphens allowed if the folder uses them).
- The **Rust crate name** for the library is the same string with `-` replaced
  by `_` (Cargo’s default for `src/lib.rs`). Example: package `acme-helper` →
  `use acme_helper::config`.
- The binary then does `use <worker_snake>::{config, functions, manifest};`
  instead of `mod config;` at the top level.

### Optional: richer layout for BDD (pattern B only)

If you use Cucumber (section 9, pattern B), also add:

- `tests/bdd.rs` — `harness = false` test entry (see section 9).
- `tests/common/` — shared test harness (`engine.rs`, `world.rs`, …).
- `tests/steps/` — step definitions.
- `tests/features/*.feature` — Gherkin scenarios.

## 3. Manifest: `iii.worker.yaml`

Shape (replace `<worker>` and the description):

```yaml
iii: v1
name: <worker>
language: rust
deploy: binary
manifest: Cargo.toml
bin: <worker>
description: One-line description shown in the registry.
```

Field-by-field rules:

- `iii: v1` — manifest schema version. Always `v1` today.
- `name` — **must equal the folder name** (regex `^[a-z0-9][a-z0-9_-]*$`). It
  is the registry record key, the consumer install command (`iii worker add
  <name>`), and the git tag prefix (`<name>/v<X.Y.Z>`).
- `language: rust` — fixed.
- `deploy: binary` — fixed for this scaffold. Drives the multi-target
  cross-compile path in CD.
- `manifest: Cargo.toml` — relative to the worker folder.
- `bin:` — **must equal** the `[[bin]].name` from `Cargo.toml`. The release
  workflow looks for this exact name in the cargo target dir.
- `description:` — single line; surfaced in the registry and `iii worker info`.

`pr-checks` parses this file and fails the PR if `name`, `language`, `deploy`,
or `manifest` are missing or unparseable.

## 4. `Cargo.toml` and `build.rs`

Minimal crate (substitute `<worker>`):

```toml
[workspace]

[package]
name = "<worker>"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "<worker>"
path = "src/main.rs"

[dependencies]
iii-sdk = "=0.19.4"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "signal"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
clap = { version = "4", features = ["derive"] }
schemars = "0.8"

[dev-dependencies]
serde_json = "1"
```

Non-obvious requirements:

- The `[workspace]` line at the top is **required**. Each worker is its own
  isolated cargo workspace; without it, cargo will walk up to the parent
  directory and try to merge with whatever it finds. The CI runner builds each
  worker from inside its own folder; do not rely on a parent workspace.
- `iii-sdk = "=0.19.4"` — pin the exact version. All workers in this repo are
  on `0.19.4`; bump in lockstep, never drift.
- The four tokio features (`rt-multi-thread`, `macros`, `sync`, `signal`) are
  the minimum to power `#[tokio::main]`, the SDK's internals, and
  `tokio::signal::ctrl_c()`. Add `time`, `fs`, or others when your handlers need
  them.
- **`schemars`** — pairs with `RegisterFunction::new_async` (section 7); derive
  `JsonSchema` on request/response structs so the SDK can emit JSON Schema for
  tools and documentation.

**Optional `[lib]`** (after `[[bin]]`, same folder):

```toml
[lib]
path = "src/lib.rs"
```

You do not need an explicit `[lib] name` — Cargo derives the library name from
`[package].name` (hyphens → underscores for Rust).

`build.rs`:

```rust
fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
```

This makes the build-time target triple (e.g. `aarch64-apple-darwin`)
available as `env!("TARGET")` from `src/manifest.rs`. The registry publish
step uses `supported_targets` to match a worker binary to the consumer's host
triple.

## 5. Config: `config.yaml` + `src/config.rs` + `src/manifest.rs`

Two paths. Pick one per worker; do not mix them in production boot.

**Path A (baseline — static config)** — for simple workers not on the
configuration worker yet (unreleased scaffolds):

- Committed `config.yaml` + `load_config()` + `--config` default `./config.yaml`
- Sections below through "Reactive config" describe Path A.

**Path B (configuration worker — preferred for production workers)** — see
[`configuration.md`](configuration.md) for the full recipe:

- **No** committed `config.yaml`
- **No** `load_config()` in the production boot path
- `--config` is `Option<String>` (no default path); optional one-time seed only
- `register_config` + `fetch_config` are **required** boot dependencies
- Choose reload tier (see [`configuration.md`](configuration.md) §6): ConfigCell
  + targeted rebuild/re-bind vs full runtime swap (+ resync). **Hot-reload is the
  default — no field should require a restart.**

Reference: `session-manager` (Path B, full runtime + resync), `context-manager`
and `approval-gate` (Path B, ConfigCell + targeted rebuild/re-bind).

### `config.yaml` (Path A only)

Operator-facing defaults, committed alongside the worker:

```yaml
default_budget: 20
max_budget: 100
timeout_per_run_ms: 30000
```

Keep it human-editable: snake_case keys, scalar values where possible, inline
comments explaining units.

### `src/config.rs`

Five required pieces:

1. **One struct** named `<Worker>Config` (e.g. `WorkerConfig`) with `#[derive(Deserialize, Debug, Clone)]`.
2. **One `serde(default = "...")` per field**, so `serde_yaml::from_str("{}")`
   produces a fully-populated struct.
3. **One `default_<field>()` function per field**, returning the literal
   default. These are the single source of truth.
4. **`impl Default`** that mirrors the `default_*()` functions. Needed by
   `manifest::build_manifest()` (which doesn't have a YAML file at hand) and
   by `main.rs`'s graceful fallback when `--config` is missing or malformed.
5. **`pub fn load_config(path: &str) -> Result<<Worker>Config>`** that does
   `std::fs::read_to_string` + `serde_yaml::from_str`. Bubble errors with
   `anyhow`.

Co-locate a `#[cfg(test)] mod tests` block covering at least:

- Empty YAML (`{}`) parses to defaults.
- Custom YAML overrides each field.
- `Default::default()` matches the YAML defaults.

This is what `cargo test` runs in CI when the engine is not available.

### `src/manifest.rs`

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "<one-line description>".to_string(),
        default_config: serde_json::json!({
            // mirror config::Config::default() field-for-field
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
```

Five required fields on the JSON output: `name`, `version`, `description`,
`default_config` (object), `supported_targets` (non-empty array). The
registry's `POST /publish` rejects manifests missing any of them.

Add a `#[cfg(test)] mod tests` that round-trips the manifest through
`serde_json::to_string_pretty` and asserts each required field is present and
non-empty.

You may **additionally** cover `--manifest` by spawning the binary from
`tests/manifest.rs` (pattern A, section 9); that is optional if unit tests
already enforce the JSON contract.

### Path B — reactive config via the `configuration` worker

When a worker needs a **schema-validated, observable, hot-reloadable** config
that other workers and the operator console can read and edit on a live bus,
use **Path B**: migrate to the built-in **`configuration` worker**. Register a
JSON Schema, fetch the authoritative value at boot, and hot-reload on change.

Path B **does not ship** `config.yaml`. Defaults live in
`WorkerConfig::default()` and are registered as `initial_value` only when
nothing is stored yet. `src/config.rs` gains `json_schema()` / `to_json` /
`from_json` / `boot_signature()`; production boot does not call `load_config()`.
See [`configuration.md`](configuration.md) for reload tiers and boot order;
`session-manager`, `context-manager`, `approval-gate`, `shell`, `storage`,
`database`, and `coder` follow Path B. `session-manager`, `context-manager`,
and `approval-gate` ship **no** `config.yaml`; some older workers still ship a
legacy seed file — do not mirror that for new integrations.

## 6. `src/main.rs`: the entry point

The entry point must:

1. **Initialise tracing**: `tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();`. Operators tune via `RUST_LOG`.
2. **Parse CLI** with clap derive. Required flags:
   - **Path A:** `--config <PATH>` (default `./config.yaml`)
   - **Path B:** `--config <PATH>` optional seed only — no default path:

     ```rust
     #[arg(long)]
     config: Option<String>,   // optional seed; no default path
     ```

   - `--url <URL>` (default `ws://127.0.0.1:49134`)
   - `--manifest` (boolean)
3. **Handle `--manifest`**: if set, print
   `serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()` and
   `return Ok(())`. **No engine connection.** This path is what the registry
   publish pipeline calls; it must be fast and side-effect-free.
4. **Load config** (Path A), falling back to defaults on error:

   ```rust
   let cfg = match config::load_config(&cli.config) {
       Ok(c) => c,
       Err(e) => {
           tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
           config::<Worker>Config::default()
       }
   };
   let cfg = std::sync::Arc::new(cfg);
   ```

   **Path B:** connect first, then `register_config(cli.config.as_deref())` +
   `fetch_config()` — see [`configuration.md`](configuration.md) §4c.

5. **Connect to iii** with **`WorkerMetadata` required** — operators and the
   registry rely on a stable identity line for your process:

   ```rust
   use iii_sdk::{register_worker, InitOptions, WorkerMetadata};

   let iii = register_worker(
       &cli.url,
       InitOptions {
           metadata: Some(WorkerMetadata {
               runtime: "rust".to_string(),
               version: env!("CARGO_PKG_VERSION").to_string(),
               name: "<worker>".to_string(), // MUST match iii.worker.yaml / folder
               os: std::env::consts::OS.to_string(),
               pid: Some(std::process::id()),
               telemetry: None,
               ..WorkerMetadata::default()
           }),
           ..InitOptions::default()
       },
   );
   let iii = std::sync::Arc::new(iii);
   ```

6. **Register custom trigger types first (when applicable)**. If your worker
   defines custom trigger types (new `trigger_type` strings) and your function
   handlers capture handles or subscriber sets produced during that
   registration, call your `register_custom_trigger_types(...)` (or equivalent)
   **before** `functions::register_all`. If you only use built-in trigger types
   (`http`, `cron`, `state`, …), skip this step.

7. **Register all functions**: `functions::register_all(&iii, &cfg, ...)` (add
   extra args only if your worker needs them).

8. **Register optional triggers** (HTTP, cron, etc.). Many workers expose some
   functions over HTTP — that is **optional**. If you register HTTP triggers,
   do it **after** functions exist:

   ```rust
   use iii_sdk::RegisterTriggerInput;

   let triggers = [
       ("<worker>::<verb>", "<worker>/<verb>", "POST"),
       // ... one tuple per HTTP-exposed function
   ];

   for (function_id, api_path, http_method) in &triggers {
       match iii.register_trigger(RegisterTriggerInput {
           trigger_type: "http".to_string(),
           function_id: function_id.to_string(),
           config: serde_json::json!({
               "api_path": api_path,
               "http_method": http_method,
           }),
           metadata: None,
       }) {
           Ok(_)  => tracing::info!(function_id, api_path, "http trigger registered"),
           Err(e) => tracing::warn!(error = %e, function_id, "failed to register http trigger"),
       }
   }
   ```

9. **Wait for SIGINT**, then shut down cleanly:

   ```rust
   tokio::signal::ctrl_c().await?;
   iii.shutdown_async().await;
   Ok(())
   ```

Do not skip the shutdown call. The SDK runs its WebSocket loop on a
background thread; the process needs to await the shutdown future or the
final messages can be lost.

## 7. Functions: registering and writing handlers

### Naming

Function IDs are `<worker>::<verb>` (e.g. `shell::exec`, `context::assemble`).
Each segment is lowercase, and **multi-word segments use kebab-case, never
snake_case** — `context::count-tokens`, `approval::list-pending`, and
`shell::on-config-change`, not `context::count_tokens` or
`approval::pending_created`. The id is the public wire surface other workers and
agents call, so renaming it later is a breaking change. The same rule applies
to a trigger's target `function_id` and to any custom `trigger_type` string
(§8) — e.g. `approval::pending-created`, not `approval::pending_created`.

### Layout

One file per function (or small feature group) under `src/functions/`.
`mod.rs` declares them and exposes a single
`pub fn register_all(iii: &Arc<III>, config: &Arc<Config>)`
(or a longer signature if you pass trigger-registration results) that calls
one private `register_<verb>` per function. Each `<verb>.rs` normally holds
**input/output structs** (`Deserialize` / `Serialize` + `JsonSchema`) and any
helpers; registration uses `RegisterFunction::new_async` in `mod.rs` or next
to the types.

### Canonical registration

Use **`RegisterFunction::new_async`** with **typed inputs and outputs**. Derive
**[`JsonSchema`](https://docs.rs/schemars)** on both: the SDK turns them into
`request_format` / `response_format` JSON Schema for tools and docs—no
hand-written `serde_json::json!` schemas.

- **Input**: `Deserialize + JsonSchema` (often `Debug` as well). Use
  **`///` doc comments on fields**; they become schema `description` strings.
- **Output**: `Serialize + JsonSchema`.
- **Capture state** the same way as any closure: clone `Arc<III>` /
  `Arc<<Worker>Config>` before `move |req: YourInput| { ... }`.

```rust
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateInput {
    /// Human-readable name for the new <thing>.
    name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CreateOutput {
    id: String,
}

fn register_create(iii: &Arc<III>, config: &Arc<<Worker>Config>) {
    let iii_inner = iii.clone();
    let cfg_inner = config.clone();
    iii.register_function(
        "<worker>::create",
        RegisterFunction::new_async(move |req: CreateInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                // … use `iii`, `cfg`, `req.name` …
                Ok::<_, IIIError>(CreateOutput {
                    id: "…".to_string(),
                })
            }
        })
        .description("Create a new <thing>."),
    );
}
```

**Parameterless payloads** (empty JSON object `{}`): use an empty struct with
`Default`:

```rust
#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListInput {}
```

Chain **`.description("…")`** on the `RegisterFunction` builder so the function
has a human-readable summary alongside the generated schemas.

**Legacy (avoid for new code):** `register_function_with` /
`RegisterFunctionMessage` and manual `request_format` / `response_format`
blobs still work, but you duplicate schema in Rust types and JSON; prefer
`RegisterFunction::new_async` + `JsonSchema` for new workers.

### Typed schemas are mandatory — never register a `Value` handler

Every function's request **and** response must be a concrete type deriving
`JsonSchema`. A closure that takes `serde_json::Value` (or returns it) makes the
SDK emit the permissive `AnyValue` schema, which ships to the registry as an
**"unknown"** request/response schema — the exact failure this rule prevents.
The reference shape is
[`iii-directory/src/functions/download.rs`](../../iii-directory/src/functions/download.rs):
a typed input struct as the closure parameter (`move |req: RepoDownloadInput|`)
plus a typed return type. (That file predates the `<worker>::<verb>` naming
convention above; copy its *schema* pattern, not its function ids.)

Apply it everywhere, including the surfaces that are easy to leave untyped:

- **No-arg calls** (`{}` payloads): use an empty struct
  `#[derive(Default, Deserialize, JsonSchema)] struct FooRequest {}`. Plain serde
  structs ignore unknown fields, so the engine-injected `_caller_worker_id` (and
  any future field) is tolerated — do **not** reach for `Value` to "ignore the
  payload".
- **Trigger-bound / internal handlers** (config-change, cron sweeps, pubsub
  subscribers): type the event payload (only the fields you read; the rest are
  ignored) and return a small typed ack (e.g. `struct Ack { ok: bool }`) instead
  of `Ok(Value::Null)`.
- **Custom bad-request errors:** if you previously hand-deserialized `Value` to
  control the error code, use
  `RegisterFunction::new_async_with_bad_request(handler, map_err)` — it
  auto-extracts the typed schema **and** lets you own the malformed-payload
  contract.
- **Genuinely freeform sub-fields** (a verbatim-forwarded `messages` array,
  provider passthrough options): keep that one field `Value` *inside* a typed
  struct. The top-level request schema stays concrete; only the nested blob is
  permissive.

This is enforced two ways (see section 9): a per-worker golden test fails if any
catalog schema is the `AnyValue`/empty schema, and the publish pipeline
(`collect_worker_interface.py --assert-typed-schemas`) re-checks the *live*
deployed interface so an untyped handler can never reach the registry.

### Handler shape

Prefer **async closures** passed to `RegisterFunction::new_async` that return
`Ok::<_, IIIError>(YourOutput { ... })`. Put shared logic in plain `async fn`
helpers if the body gets large.

Serde performs shape validation when building `YourInput`; map failures to
`IIIError::Handler` if you need a specific message, or let the SDK surface
deserialization errors when that is acceptable.

Rules:

- **Errors** are `IIIError::Handler(String)` (or other `IIIError` variants when
  applicable). The engine surfaces the string to the caller.
- **Business validation** (`name` empty, cross-field rules, etc.): return
  `Err(IIIError::Handler("…".into()))` after deserialize succeeds.
- **Numbers**: with typed structs, use normal Rust numeric fields (`u64`,
  `f64`, …) and serde; reserve ad-hoc `serde_json::Value` for truly dynamic
  blobs.
- **No panics in handlers.** Every `?` should resolve to an `IIIError`.

### Calling other functions

A handler invokes any function (built-in or another worker's) via
`iii.trigger`. Example `state::get` wrapper:

```rust
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

pub async fn state_get(iii: &III, scope: &str, key: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}
```

Other built-ins worth knowing: `state::set`, `state::delete`, `state::list`,
`stream::set`, `stream::update`, `stream::list`. They all follow the same
`TriggerRequest` shape.

## 8. Triggers: types and configs

A function is callable via the SDK on its own. To make it callable from
outside iii (HTTP, cron, queue, etc.) you also register a **trigger** that
binds an external event source to the function.

**Naming.** A trigger's `function_id` is the id the function was registered
under, so it follows the same kebab-case rule as functions (§7 Naming). Any
custom `trigger_type` string you introduce is also kebab-case (e.g.
`storage::object-created`), never snake_case.

Trigger types in use across this repo and the SDK's built-ins:

| Trigger type | Config keys | Used for |
|---|---|---|
| `http` | `api_path`, `http_method` | Expose a function as an HTTP endpoint at `<engine_http>/<api_path>` |
| `cron` | `expression` (6-field cron, e.g. `"0 */5 * * * *"`) | Periodic invocation |
| `durable:subscriber` | `topic`, `queue_config?` | Drain a named durable queue topic. The type string is `durable:subscriber` — `queue` is the provider *package*, not a trigger type |
| `subscribe` | `topic` | Fan-out on a pubsub topic |
| `state` | `scope?`, `key?` | Reactive: fire when a state key changes (omit `key` and it fires for every key in the scope) |
| `stream` | `stream_name?`, `group_id?`, `item_id?` | React to stream item events |
| `stream:join` / `stream:leave` | `stream_name?` | React to stream group join / leave |
| `log` | `level?` | Log events at or above a level |
| `trace` | `service_name?`, `status?` | Spans from a service / with a status |
| `configuration` | `configuration_id?`, `event_types?` | A configuration value changed |

**Conditions.** Every builtin type above except `log` and `trace` also accepts
`condition_function_id` in its config. Before dispatching, the engine calls
that function with the event as its whole payload: a bare `false` skips the
handler, any other value (or no result) proceeds, and a condition that
**errors** — unknown id, payload the target can't parse — also skips, silently
and forever. The condition sees only the event payload, so gating on external
state means naming a function that fetches it. Worker-defined trigger types
(the custom ones you register with `register_trigger_type`) are dispatched by
your own worker and never consult a condition: the key rides along as inert
metadata.

Example HTTP trigger:

```rust
iii.register_trigger(RegisterTriggerInput {
    trigger_type: "http".to_string(),
    function_id: "<worker>::create".to_string(),
    config: serde_json::json!({
        "api_path": "<worker>/create",
        "http_method": "POST",
    }),
    metadata: None,
})?;
```

If you use HTTP: **for every function that should be callable over HTTP,
register both the function (`RegisterFunction::new_async` / `iii.register_function`)
AND the trigger (`register_trigger`) with `trigger_type: "http"`**, and document
the API in the README if end users call it.

`register_trigger` returns a `Result` and the failure mode is usually
“another worker already registered the same `api_path`”. Log and continue,
don't abort the worker — use a `match` as in section 6.

## 9. Tests

CI runs `cargo test --all-features` in each worker folder. `pr-checks`
additionally requires `tests/` to exist and be non-empty.

**`CARGO_BIN_EXE_*`:** Cargo exposes `env!("CARGO_BIN_EXE_<BIN>")` where `<BIN>`
is the **binary name** with `-` replaced by `_` (e.g. `acme-helper` →
`CARGO_BIN_EXE_acme_helper`).

### Wire-surface catalog + golden schema test (required)

The set of `{function_id, request_schema, response_schema}` a worker registers is
its agent-facing **product surface**. Pin it so every change is an explicit,
reviewed diff and a `Value` handler can never sneak in:

1. Expose a `pub fn catalog() -> Vec<FunctionSpec>` (in `src/surface.rs`, or
   `src/functions/mod.rs`) listing every registered function, in registration
   order, with its typed request/response structs. Build each `RootSchema` with
   the **same** generator the SDK uses, so the snapshot equals what registration
   emits:

```rust
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}
```

2. Add `tests/schemas.rs` (plus a `tests/support/mod.rs` golden harness) that:
   - serializes each catalog entry to `tests/golden/schemas/<id>.json` (`::` maps
     to `.`) and compares it against the committed golden;
   - asserts the catalog ids match registration order;
   - asserts **every** request/response schema is typed — i.e. it carries a
     schema-defining keyword (`type` / `properties` / `$ref` / `anyOf` / …) and is
     never the `AnyValue`/empty schema.

   Copy the harness and the `assert_typed_schema` helper verbatim from an
   existing worker (`context-manager`, `approval-gate`, `llm-router`,
   `session-manager`, `provider-openai`).

3. Regenerate goldens with `UPDATE_GOLDENS=1 cargo test`; review the diff and
   commit the goldens alongside the change that caused them.

The publish pipeline is the second line of defense: it collects the live
interface from `engine::functions::list` and runs
`collect_worker_interface.py --assert-typed-schemas`, failing the release if any
deployed function still has an `AnyValue`/empty ("unknown") schema. The Rust test
catches it at PR time; the pipeline check is the backstop at publish time.

### Pattern A — minimal: manifest subprocess + integration

Good default for small workers: fast manifest check, one subprocess e2e.

**`tests/manifest.rs`** — spawn `--manifest` and validate JSON:

```rust
use std::process::Command;
use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_<worker_underscored>");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn <worker> --manifest");

    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");

    assert_eq!(manifest["name"], env!("CARGO_PKG_NAME"));
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        manifest["default_config"].is_object(),
        "default_config must be an object"
    );
    assert!(
        !manifest["supported_targets"]
            .as_array()
            .expect("supported_targets must be an array")
            .is_empty(),
        "supported_targets must not be empty"
    );
}
```

Replace `CARGO_BIN_EXE_<worker_underscored>` with the actual compile-time
symbol for your `[[bin]].name`.

Add to `Cargo.toml` **for pattern A** (integration):

```toml
[dev-dependencies]
serde_json = "1"
which = "8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "signal"] }
```

**`tests/integration.rs`** — spawn `iii` + worker, drive via SDK; **self-skip**
when `iii` is not on `PATH`:

```rust
//! Spawn the `iii` engine and the worker binary as subprocesses, then drive
//! the worker through `iii-sdk` as a client. Self-skips when `iii` is absent.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{InitOptions, TriggerRequest, register_worker};
use serde_json::json;
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

struct Harness {
    iii: Child,
    worker: Child,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
    }
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_<worker_underscored>");

    // Path A: pass --config with a test fixture or config.yaml.
    // Path B (configuration worker): omit --config unless testing seed
    // semantics — the engine's configuration worker holds authoritative config.
    let worker = Command::new(worker_bin)
        .arg("--url")
        .arg(ENGINE_WS)
        // .args(["--config", &config_path])   // Path A only
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(1500)).await;

    Some(Harness { iii, worker })
}

#[tokio::test]
async fn end_to_end_via_iii_sdk() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let result = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "<worker>::<verb>".into(),
            payload: json!({ /* … */ }),
            action: None,
            timeout_ms: Some(5000),
        }),
    )
    .await
    .expect("trigger timed out")
    .expect("trigger failed");

    assert!(result.is_object());

    client.shutdown_async().await;
}
```

Required behaviours:

- **Boot order**: `iii` first, then the worker.
- **`Drop for Harness`** kills both processes when the test ends.
- **`stdout(Stdio::null())`** keeps `cargo test` readable; use
  `Stdio::inherit()` while debugging.
- **Self-skip when `iii` is missing**.
- **The test client's `register_worker` does not register functions** — only
  `trigger`.
- **Wrap each `trigger` in `tokio::time::timeout`**.

For HTTP-triggered functions you can additionally hit the engine HTTP API
(default `http://127.0.0.1:3000`).

### Pattern B — Cucumber (BDD)

Use this when scenarios read better as Gherkin or when you need many
engine-backed flows with shared setup.

**`Cargo.toml`** additions:

```toml
[dev-dependencies]
cucumber = "0.22"
futures = "0.3"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "signal", "time"] }

[[test]]
name = "bdd"
harness = false
```

**`tests/bdd.rs`** (entry point; Cucumber owns `main`):

```rust
mod common;
mod steps;

use cucumber::World;

use crate::common::world::MyWorld;

#[tokio::main]
async fn main() {
    MyWorld::cucumber()
        .max_concurrent_scenarios(1)
        .before(|_f, _r, _s, world| {
            Box::pin(async move {
                // reset shared state, optional engine connect, etc.
                let _ = world;
            })
        })
        .run_and_exit("tests/features")
        .await;
}
```

**`tests/common/mod.rs`**

```rust
pub mod world;
```

**`tests/common/world.rs`** (minimal stub — expand for your worker):

```rust
use cucumber::World;

#[derive(Debug, Default, World)]
pub struct MyWorld {
    pub stash: std::collections::HashMap<String, String>,
}
```

**`tests/steps/mod.rs`**

```rust
mod example_steps;
```

**`tests/steps/example_steps.rs`**

```rust
use cucumber::{given, then, when};

use crate::common::world::MyWorld;

#[given("the worker is clean")]
async fn clean(world: &mut MyWorld) {
    world.stash.clear();
}

#[when("we record {word} = {word}")]
async fn record(world: &mut MyWorld, key: String, val: String) {
    world.stash.insert(key, val);
}

#[then("stash has {word}")]
async fn assert_key(world: &mut MyWorld, key: String) {
    assert!(world.stash.contains_key(&key));
}
```

**`tests/features/smoke.feature`**

```gherkin
Feature: smoke

  @pure
  Scenario: stash round trip
    Given the worker is clean
    When we record greeting = hello
    Then stash has greeting
```

Run:

```bash
cargo test --test bdd
cargo test --test bdd -- --tags @pure
```

For engine scenarios, add a `tests/common/engine.rs` that connects once,
share the `Arc<III>` on the `World`, and tag scenarios `@engine` so hosts
without `iii` can filter them out.

## 10. README contract

`pr-checks` requires `README.md` to exist and be non-empty; its body becomes
the registry's `readme` field on `POST /publish`.

**Consumer-facing structure** (section order, tone, what belongs in README vs
reference tooling) is defined in [`worker-readme.md`](../../worker-readme.md). Follow
that guide for headings like Install, Quickstart, and Configuration.

Avoid front-loading huge function/schema tables: `iii worker info`, rustdoc,
and editor integrations already expose function ids. Optional `## Functions`
or similar is fine when it helps operators, but keep it concise.

This document does **not** duplicate the full README skeleton — use
`worker-readme.md` for prose patterns and paste-ready markdown fences.

## 11. Verification checklist

Before opening a PR, run from the worker folder:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./target/debug/<worker> --manifest | jq .
```

Use the same `<worker>` string as `[[bin]].name` (hyphens preserved in the
filename). For pattern B also run `cargo test --test bdd`.

The checks:

- `fmt` — must pass with no diff. CI runs the same command.
- `clippy` — `-D warnings` is the same level CI enforces.
- `test` — passes locally and on CI hosts that don't have `iii` (e2e tests
  should self-skip or use `@pure`-only runs as you designed). This includes the
  wire-surface golden test (`tests/schemas.rs`): every function must have a typed
  request/response schema (no `AnyValue`/"unknown"). If you changed a function's
  input/output, regenerate with `UPDATE_GOLDENS=1 cargo test` and commit the
  golden diff.
- `--manifest` — valid JSON with `name`, `version`, `description`,
  `default_config`, `supported_targets`.

## 12. Copy-paste skeleton

Create the worker folder and drop these files in. Replace `<worker>` (the
folder name) and `<Worker>` (PascalCase) everywhere; pick a single placeholder
function name like `echo` or `ping` for the first iteration.

### `<worker>/iii.worker.yaml`

```yaml
iii: v1
name: <worker>
language: rust
deploy: binary
manifest: Cargo.toml
bin: <worker>
description: One-line description shown in the registry.
```

### `<worker>/Cargo.toml`

```toml
[workspace]

[package]
name = "<worker>"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "<worker>"
path = "src/main.rs"

[dependencies]
iii-sdk = "=0.19.4"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "signal"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
clap = { version = "4", features = ["derive"] }
schemars = "0.8"

[dev-dependencies]
serde_json = "1"
which = "8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "signal"] }
```

### `<worker>/build.rs`

```rust
fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
```

### `<worker>/config.yaml` (Path A only; omit for Path B)

```yaml
greeting: "hello"
```

### `<worker>/src/main.rs`

```rust
use anyhow::Result;
use clap::Parser;
use iii_sdk::{InitOptions, RegisterTriggerInput, WorkerMetadata, register_worker};
use serde_json::json;
use std::sync::Arc;

mod config;
mod functions;
mod manifest;

#[derive(Parser, Debug)]
#[command(name = "<worker>", about = "<one-line description>")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::WorkerConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "<worker>".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);
    functions::register_all(&iii, &cfg);

    let triggers = [("<worker>::echo", "<worker>/echo", "POST")];

    for (function_id, api_path, http_method) in &triggers {
        match iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: function_id.to_string(),
            config: json!({ "api_path": api_path, "http_method": http_method }),
            metadata: None,
        }) {
            Ok(_) => tracing::info!(function_id, api_path, "http trigger registered"),
            Err(e) => tracing::warn!(error = %e, function_id, "failed to register http trigger"),
        }
    }

    tracing::info!("<worker> ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("<worker> shutting down");
    iii.shutdown_async().await;
    Ok(())
}
```

### `<worker>/src/config.rs`

```rust
use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct WorkerConfig {
    #[serde(default = "default_greeting")]
    pub greeting: String,
}

fn default_greeting() -> String {
    "hello".to_string()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { greeting: default_greeting() }
    }
}

pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: WorkerConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.greeting, "hello");
    }

    #[test]
    fn custom_yaml_overrides() {
        let cfg: WorkerConfig = serde_yaml::from_str("greeting: hi").unwrap();
        assert_eq!(cfg.greeting, "hi");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        assert_eq!(WorkerConfig::default().greeting, "hello");
    }
}
```

### `<worker>/src/manifest.rs`

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "<one-line description>".to_string(),
        default_config: serde_json::json!({
            "greeting": "hello"
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["default_config"].is_object());
        assert!(parsed["supported_targets"].as_array().unwrap().len() >= 1);
    }
}
```

### `<worker>/src/functions/mod.rs`

```rust
pub mod echo;

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};

use crate::config::WorkerConfig;

pub fn register_all(iii: &Arc<III>, config: &Arc<WorkerConfig>) {
    register_echo(iii, config);
    tracing::info!("all functions registered");
}

fn register_echo(iii: &Arc<III>, config: &Arc<WorkerConfig>) {
    let cfg = config.clone();
    iii.register_function(
        "<worker>::echo",
        RegisterFunction::new_async(move |req: echo::EchoInput| {
            let cfg = cfg.clone();
            async move {
                Ok::<_, IIIError>(echo::EchoOutput {
                    message: format!("{}, {}!", cfg.greeting, req.name),
                })
            }
        })
        .description("Echo the input prefixed with the configured greeting."),
    );
}
```

### `<worker>/src/functions/echo.rs`

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EchoInput {
    /// Name to include in the echoed message.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EchoOutput {
    pub message: String,
}
```

### `<worker>/tests/manifest.rs`

```rust
use std::process::Command;

use serde_json::Value;

#[test]
fn manifest_subcommand_emits_valid_json() {
    let bin = env!("CARGO_BIN_EXE_<worker_underscored>");
    let output = Command::new(bin)
        .arg("--manifest")
        .output()
        .expect("spawn <worker> --manifest");

    assert!(
        output.status.success(),
        "binary exited with {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("manifest stdout is utf-8");
    let manifest: Value = serde_json::from_str(&stdout).expect("manifest stdout is valid JSON");

    assert_eq!(manifest["name"], env!("CARGO_PKG_NAME"));
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert!(manifest["default_config"].is_object());
    assert!(
        !manifest["supported_targets"]
            .as_array()
            .expect("supported_targets must be an array")
            .is_empty()
    );
}
```

### `<worker>/tests/integration.rs`

```rust
//! End-to-end test: spawn `iii` engine + the worker binary, drive via SDK as a client.
//! Self-skips when `iii` is not on PATH.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{InitOptions, TriggerRequest, register_worker};
use serde_json::json;
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

struct Harness {
    iii: Child,
    worker: Child,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
    }
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_<worker_underscored>");

    // Path A: pass --config. Path B: omit unless testing seed semantics.
    let worker = Command::new(worker_bin)
        .arg("--url")
        .arg(ENGINE_WS)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(1500)).await;

    Some(Harness { iii, worker })
}

#[tokio::test]
async fn end_to_end_via_iii_sdk() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let result = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "<worker>::echo".into(),
            payload: json!({ "name": "world" }),
            action: None,
            timeout_ms: Some(5000),
        }),
    )
    .await
    .expect("trigger timed out")
    .expect("trigger failed");

    assert_eq!(result["message"], "hello, world!");

    client.shutdown_async().await;
}
```

### `<worker>/README.md`

Use [`worker-readme.md`](../../worker-readme.md) for section order and consumer tone.
Minimum viable shape (use a four-backtick outer fence when the README embeds
code blocks):

````markdown
# <worker>

One paragraph: what the worker does, who calls it, and the single
most important thing it gives you.

## Install

```bash
iii worker add <worker>
```

## Run

```bash
iii start
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii.trigger(TriggerRequest {
        function_id: "<worker>::echo".into(),
        payload: json!({ "name": "world" }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    println!("{result:#?}");
    Ok(())
}
```

## Configuration

```yaml
greeting: "hello"   # prefix used by <worker>::echo
```

## Local development & testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
cargo test
```
````

That's the full skeleton for pattern A. Run section 11 before publishing.
