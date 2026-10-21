# Sandbox worker: shared interface registry

Reference for the six plans built from `docs/superpowers/specs/2026-10-20-sandbox-worker-design.md`. Plans use these names and types verbatim; a plan that needs a change edits this file first. **(spec)** = fixed by the spec; **(registry)** = chosen here. iii paths are at `a1d08d8b0`, workers paths at `8773952be`. Items the spec cuts from v1 (its section 2) appear only as "not in v1". `(Dn)` cites the coordinator's plan triage decisions (plan index), which override drafts and spec text.

## 1. Conventions

| Item | Value |
|---|---|
| Contract crate | `iii-sandbox-contract`, lib `iii_sandbox_contract` (spec). Internal: `publish = false`, used by path by the engine (P2a) and Compose (P2b), never published (spec Q5; supersedes D1, D2, D4). `version.workspace = true`, `license = "Elastic-2.0"` like the other iii crates, edition 2024, `rust-version = "1.85"`. Deps `serde 1`, `serde_json 1`, `schemars 0.8` (spec; 0.8 as `engine/Cargo.toml:110`); dev-dep `iii-sdk = { workspace = true }` (spec) |
| Engine, Compose | `iii` 0.25.0 is cut at rollout step 4 by `create-tag.yml`; no plan edits engine versions. `iii-compose` stays 0.1.0, path dep (registry) |
| `sandbox-builtin` | 1.0.0 (spec S2); `license = "Elastic-2.0"`, the moved code's license; `iii-sdk = "=0.24.6"`, `iii-helpers = "=0.24.6"` (spec 8.1); no `iii-sandbox-contract` dependency: its own copies of the §2 types (spec Q5) |
| Commits, PRs | conventional (`feat(sandbox): …`), MOT ticket in the PR title, never an AI co-author or "Generated with" line |

| Plan | Repo | Branch (registry) | Base |
|---|---|---|---|
| P0 | iii | `sandbox/p0-contract` | `a1d08d8b0` |
| P1 | workers | `sandbox/p1-builtin` | `origin/main` (`8773952be` + one harness-only commit); independent of the iii repo (Q5) |
| P2a / P2b / P3 | iii | `sandbox/p2a-router` / `sandbox/p2b-compose` / `sandbox/p3-engine-cleanup` | main after P0 / P2a / P2b merge. No stable tag from main between P2a's merge and 0.25.0: stable tags come from main (`create-tag.yml:89-92`), and a patch would ship the router before Compose binds it. P3 cannot merge before `sandbox-builtin` 1.0.0 is in the registry |
| P5 | workers | `sandbox/p5-consumers` | main after P1 merges and engine 0.25.0 ships |

## 2. Contract crate API (P0 produces; P2a and P2b consume; P1 keeps copies with these names, Q5)

Files (registry): `crates/iii-sandbox-contract/src/{lib.rs, operations.rs, channel.rs, errors.rs, info.rs, ephemeral.rs, worker.rs}`; tests `tests/{wire_parity.rs, stream_channel_ref.rs, fixtures/sandbox_error_codes.json, fixtures/wire_examples.json}` (`wire_examples.json`: `{"<TypeName>": <example>}` for every info and worker-VM type; P1 copies both fixtures and round-trips them, Q5); workspace member in `Cargo.toml:4-22`. `lib.rs` re-exports every item at the root: consumers write `iii_sandbox_contract::ExecRequest`.

```rust
pub const CONTRACT_MAJOR: u32 = 1;                           // (spec)
pub const CONTRACT_MINOR: u32 = 0;                           // (spec)
pub const PUBLIC_PREFIX: &str = "sandbox::";                 // (registry)
pub const DEFAULT_ADAPTER: &str = "sandbox-builtin";         // (registry name; value spec S1)
pub const ADAPTER_INFO_SUFFIX: &str = "adapter::info";       // (registry name; id spec 5.2)
pub const ROUTER_INFO_ID: &str = "sandbox::adapter::info";   // (registry name; id spec 5.2)
/// ("sandbox-builtin", "sandbox::fs::ls" | "fs::ls") -> "sandbox-builtin::fs::ls"; strips a leading PUBLIC_PREFIX.
pub fn adapter_function_id(adapter: &str, id: &str) -> String;     // (spec)
/// `[a-z0-9-]+` (spec 6.3) and not "sandbox", which would forward the router to itself (registry).
pub fn is_valid_adapter_name(name: &str) -> bool;                  // (registry)
#[derive(Debug, Clone, Copy)] pub struct Operation {               // (registry; columns spec 5.1)
    pub suffix: &'static str,                    // "create", "fs::ls"
    pub description: &'static str,               // today's daemon registration text
    pub request_schema: fn() -> serde_json::Value,   // to_value(schema_for!(T)), as engine/function-macros/src/lib.rs:350-360
    pub response_schema: fn() -> serde_json::Value }
impl Operation { pub fn public_id(&self) -> String; }   // PUBLIC_PREFIX + suffix
pub const OPERATIONS: &[Operation];   // (spec) only the 16 ephemeral ids, in daemon order; no Family enum (D3)
pub const WORKER_PREPARE: &str = "worker::prepare"; pub const WORKER_START: &str = "worker::start";     // (D3; names registry)
pub const WORKER_STATUS: &str = "worker::status";   pub const WORKER_LOGS: &str = "worker::logs";       // worker-VM suffixes, called as
pub const WORKER_STOP: &str = "worker::stop";       pub const WORKER_DESTROY: &str = "worker::destroy"; // adapter_function_id(adapter, WORKER_START)
```
No id parser and no schema bundle (spec 2). `adapter::info` is not an `OPERATIONS` row: its two ids reply with different types.

**Ephemeral types** (spec: today's names and shapes, moved; sources relative to `crates/iii-worker/src/sandbox_daemon/`)

| Source | Types |
|---|---|
| `create.rs:18-65`, `exec.rs:24-157` | `CreateRequest`, `CreateResponse`, `EnvShape`, `ExecRequest`, `ExecResponse` (`exit_code: Option<i32>`) |
| `stop.rs:9-20`, `list.rs:11-35` | `StopRequest`, `StopResponse`, `ListRequest`, `SandboxSummary`, `ListResponse` |
| `catalog.rs:73-104`, `run.rs:47-117` | `CatalogListRequest`, `CatalogEntry`, `CatalogEntryKind`, `CatalogListResponse`, `RunFile`, `RunRequest`, `RunResponse` |
| `fs/*.rs` | `{Ls,Stat,Mkdir,Rm,Chmod,Mv,Grep,Sed,Write,Read}{Request,Response}`, `WriteContent` |
| `crates/iii-shell-proto/src/lib.rs:76-106` | copies of `FsEntry`, `FsMatch`, `FsSedFileResult` (spec); `FsReadMeta` is not copied (spec) |

Changes (registry unless marked): every request and response derives `Debug, Clone, Serialize, Deserialize, JsonSchema` (today requests are `Deserialize`-only and responses `Serialize`-only, e.g. `exec.rs:86, 147`); `EnvShape::into_kv_vec(self) -> Result<Vec<String>, String>`, whose `Err` is today's exact S001 text for the adapter to wrap; `ExecRequest.workdir` doc says "`/` when omitted" (spec 5.1); docs naming `engine.workers.iii-sandbox.custom_images` (`create.rs:22`, `catalog.rs:93`) say "the adapter's `custom_images` config"; `impl From<FsEntry> for StatResponse` moves. Handlers, the traits `VmLauncher`, `ShellRunner`, `VmStopper`, `FsRunner`, `BootParams`, `SandboxError` and its `Display` wrapper `SandboxErrorWire` stay in the adapter.

```rust
// channel.rs: same JSON as sdk/packages/rust/iii/src/channels.rs:15-28, pinned by tests/stream_channel_ref.rs (spec)
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StreamChannelRef { pub channel_id: String, pub access_key: String, pub direction: ChannelDirection }
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)] #[serde(rename_all = "lowercase")]
pub enum ChannelDirection { #[default] Read, Write }

// info.rs and worker.rs: every type derives Debug, Clone, Serialize, Deserialize, JsonSchema
pub struct ContractVersion { pub major: u32, pub minor: u32 }   // (registry name; fields spec 5.7)
impl ContractVersion { pub const CURRENT: Self; }               // { CONTRACT_MAJOR, CONTRACT_MINOR }
pub struct AdapterInfoRequest {}                                // (registry)
pub struct AdapterInfo { pub adapter: String, pub adapter_version: String, pub contract: ContractVersion } // (spec)
pub struct RouterInfo {                                         // (registry name; fields spec 5.2)
    pub adapter: Option<String>,    // None when unbound; Compose never reads RouterInfo (D11)
    pub namespace: Option<String>,  // None when unbound
    pub contract: ContractVersion,  // the engine's CURRENT
    pub info: Option<AdapterInfo> } // None when unbound or the adapter is not connected

pub struct WorkerPrepareRequest { pub auth: String, pub source: WorkerSource }    // (spec fields)
pub struct WorkerPrepareResponse {}                                               // (registry)
pub struct WorkerStartRequest { pub auth: String, pub state_key: String, pub owner: Owner,
    pub worker_name: String, pub source: WorkerSource, pub env: BTreeMap<String, String>,
    pub engine_url: String, pub network: bool }      // (spec; no resources: the adapter reads the manifest, D3)
#[serde(tag = "kind", rename_all = "snake_case")]                                 // (registry)
pub enum WorkerSource { Bundle { package: String, host_path: String },            // (spec)
    Local { host_path: String, run_override: Option<String> } }  // no sync: local sources always sync (D3)
pub struct Owner { pub kind: String /* "compose" */, pub id: String, pub container: String } // (spec)
pub struct WorkerStartResponse { pub vm_id: String, pub instance: u64 }           // (spec)
pub struct WorkerStatusRequest { pub auth: String, pub vm_id: String, pub instance: u64 } // (spec fields)
pub struct WorkerStatus { pub vm_id: String, pub instance: u64, pub state: State, pub exit_code: Option<i32>,
    pub signal: Option<i32>, pub started_at_ms: u64, pub ended_at_ms: Option<u64> } // (spec)
#[serde(rename_all = "snake_case")] pub enum State { Preparing, Running, Exited, Stopped, Unknown } // (spec)
pub struct WorkerLogsRequest { pub auth: String, pub vm_id: String, pub cursor: Option<u64>,
    pub follow: bool, pub channel: StreamChannelRef }        // (spec fields; cursor type registry)
pub struct WorkerLogsResponse {}                                                  // (registry)
pub struct LogRecord { pub stream: LogStream, pub line: String, pub ts_ms: u64, pub cursor: u64 } // (spec fields)
#[serde(rename_all = "snake_case")] pub enum LogStream { Stdout, Stderr }   // (registry; JSON as compose logs.rs:44)
pub struct WorkerStopRequest { pub auth: String, pub vm_id: String, pub instance: u64, pub grace_ms: u64 } // (spec)
pub struct WorkerDestroyRequest { pub auth: String, pub state_key: String }       // (spec)
pub struct WorkerDestroyResponse { pub destroyed: bool }                          // (registry)
```
`worker::logs`: Compose creates the channel and passes its writer ref; the adapter writes one `LogRecord` JSON per text frame, returns once the pump starts, and closes the channel at the end (`follow: false`) or when that instance ends (`follow: true`). The cursor is opaque to Compose (registry). Not in v1 (spec 2): `ArchiveRef`, an `Effective` resources reply, `EphemeralCaps`, `WorkerVmCaps`, leases; cut by D3: `Family`, `Resources`, `WorkerStartRequest.resources`, `WorkerSource::Local.sync`.

```rust
// errors.rs
pub const DOCS_BASE: &str = "https://iii.dev/docs/creating-workers/sandboxes#";  // (registry value; spec: on iii.dev)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum SandboxErrorCode { S001, S002, S003, S004, S100, S101, S102, S200, S210, S211, S212, S213, S214,
    S215, S216, S217, S218, S219, S300, S301, S302, S303, S304, S400, S501, S502, S503, S504, S505, S506 }
    // variant = code, as sandbox_daemon/errors.rs:22-43 (codes spec)
impl SandboxErrorCode { pub fn as_str(&self) -> &'static str; pub fn error_type(&self) -> &'static str;
                        pub fn retryable(&self) -> bool; }                     // today's signatures
/// {type, code, message, docs_url: DOCS_BASE + code, retryable, fix, fix_note}: the shape of errors.rs:263-277.
pub fn error_payload(code: SandboxErrorCode, message: &str, fix: Option<serde_json::Value>,
                     fix_note: Option<&str>) -> serde_json::Value;   // (registry; the spec 5.1 "message builder")
/// Parses an ErrorBody.message and returns its `code`; None if it is not an S-code payload.
pub fn code_of(message: &str) -> Option<SandboxErrorCode>;          // (registry)
```
Wire (spec 5.6): `ErrorBody { code: "invocation_failed", message: error_payload(..).to_string() }`. Parity tests compare parsed JSON, not bytes; `docs_url` changes on purpose. P0 adds `<a id="Sxxx"></a>` anchors for every code to `docs/next/creating-workers/sandboxes.mdx` (D12: `docs/next` only), with its own anchor test; P3 keeps them. P2a deletes `crates/iii-worker/tests/sandbox_docs_anchor_stability.rs` when it moves the README (D10). The fixture gains the rows below with `"rust_variant": null`; S500 stays reserved with no variant (spec).

| Code: meaning (spec 5.6) | `type` / retryable (registry; retryable flags spec) | Emitted by |
|---|---|---|
| S301 AdapterUnavailable | `transient` / true | router |
| S302 ContractMismatch, S303 NoAdapterConfigured | `config` / false | router (S302 also Compose's start check) |
| S304 OperationUnsupported | `platform` / false | an adapter lacking an op (the builtin never) |
| S501 SourceRejected, S502 StateBusy, S503 InstanceMismatch, S504 VmNotFound, S505 Unauthorized | `validation` / false | adapter `worker::*` |
| S506 PrepareFailed | `transient` / true | adapter `worker::prepare` |

## 3. Function ids

| Id | Namespace | Registered by | Request -> response |
|---|---|---|---|
| `sandbox::{create, exec, stop, list, catalog::list, run}`, `sandbox::fs::{ls, stat, mkdir, rm, chmod, mv, grep, sed, write, read}` (spec) | `default` | router (P2a) | the ephemeral types; payload forwarded untouched |
| `sandbox::adapter::info` (spec) | `default` | router | `AdapterInfoRequest` -> `RouterInfo`; never S301 or S303 (registry) |
| `<adapter>::<suffix>` for the 16 above (spec) | binding namespace | adapter (P1) | same as the public id |
| `<adapter>::adapter::info` (spec) | adapter's namespace | adapter | `AdapterInfoRequest` -> `AdapterInfo` |
| `<adapter>::worker::{prepare, start, status, logs, stop, destroy}` (spec; suffixes `WORKER_*`) | project namespace | adapter; only Compose calls them | `Worker{Prepare,Start,Status,Logs,Stop,Destroy}Request` -> `WorkerPrepareResponse`, `WorkerStartResponse`, `WorkerStatus`, `WorkerLogsResponse`, `WorkerStatus`, `WorkerDestroyResponse` |

Reserved: `<adapter>::worker::{exec, sync, wait, list, renew}` are not registered (spec; documented, no constant). `sandbox::*` in `default` belongs to the router; other namespaces stay open (spec 6.2).

## 4. Engine router API (P2a produces; P2b consumes)

- Module `engine/src/workers/sandbox/{mod.rs, config.rs, router.rs, iii.worker.yaml, README.md, skills/SKILL.md}` (spec), declared `pub mod sandbox;` in the `workers` block of `engine/src/lib.rs:25-45`. P2a moves `sandbox_daemon/{iii.worker.yaml, README.md, skills/SKILL.md}` here (D10); the adapter ships no skill (D8).
- `pub struct SandboxRouter` implements `Worker` (`engine/src/workers/traits.rs:54`) with `name() == "iii-sandbox"`; `mod.rs` holds `crate::register_worker!("iii-sandbox", SandboxRouter, description = "...", mandatory)` (spec). One handler type serves every id through `FunctionHandler::handle_function` (`engine/src/function.rs:75-83`).
- Registers the 16 `OPERATIONS` rows (`public_id()`, `description`, both schemas) and `ROUTER_INFO_ID` through `EngineTrait::register_function` (`engine/src/engine/mod.rs:325`), also when unbound (spec).
- Forward: `call_value_with_metadata_ns(&ns, &adapter_function_id(&adapter, &function_id), input, metadata)` (`mod.rs:2543`, `pub(crate)`), result unchanged (spec).
- Handshake: `<adapter>::adapter::info` on the first forward, cached. Major differs -> S302 (spec); `adapter::info` missing, or `function_not_found` from the forward -> S301; a reply that does not parse -> S302 (registry). The cache clears only on S301 or `invocation_stopped` (spec; a known limit the router README states, D10).

```rust
// engine/src/workers/sandbox/config.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] #[serde(deny_unknown_fields)]
pub struct SandboxRouterConfig {                 // (spec name)
    pub adapter: String,                         // required, is_valid_adapter_name (registry: no default in the engine)
    #[serde(default = "default_namespace")]
    pub namespace: String }                      // default "default" (registry); iii_compose::namespace::check (spec)
impl SandboxRouterConfig { pub fn validate(&self) -> anyhow::Result<()>; }   // (registry)
// engine/src/workers/config.rs: EngineConfig (:57-68) gains
#[serde(default)] pub sandbox: Option<SandboxRouterConfig>,                  // (spec)
/// workers ++ modules, inject missing mandatory entries (iii-sandbox gets `config: sandbox.map(to_value)`),
/// then assign_instance_ids. Replaces config.rs:711-729 (EngineBuilder::build) and reload.rs:125-142.
pub fn normalize_worker_entries(config: EngineConfig) -> Vec<WorkerEntry>;  // (spec name; signature registry)
// engine/src/engine/mod.rs, beside the engine:: check at :2297; applies in `default` only
const ENGINE_RESERVED_PREFIXES: &[&str] = &[iii_sandbox_contract::PUBLIC_PREFIX];   // (spec)
```
- `validate()` runs right after the `validate_declared_workers` call (`config.rs:149`). Messages (code spec, text registry): `[INVALID_SANDBOX_CONFIG] sandbox.adapter '<x>' must match [a-z0-9-]+ and must not be 'sandbox'`; `[INVALID_SANDBOX_CONFIG] sandbox.namespace '<x>': <check reason>`. Structural errors (unknown key, missing `adapter`) may surface as the existing config parse error instead (D10).
- Reserved prefix: reply `RegistrationRejected { code: FUNCTION_NAMESPACE_CONFLICT, owner_worker_id: "" }`, log "sandbox::* is served by the engine's sandbox router; adapters register <adapter>::*" (spec).
- `migration_guidance` (`config.rs:206`) arm (text registry): `"iii-sandbox" => "remove it; set sandbox.adapter (Compose: engine.sandbox) and configure the sandbox-builtin worker"`. It fires only once `"iii-sandbox"` leaves `CONFIGURABLE_ENGINE_WORKERS`, which the engine re-exports (`config.rs:22`) and checks (`config.rs:160`), so **P2a removes that entry** and, in the same PR, updates the Compose fixtures that declare `engine.workers.iii-sandbox` (`crates/iii-compose/src/managed_engine.rs`, `crates/iii-compose/tests/config_validation.rs`). P2b owns the Compose texts (`UnsupportedEngineWorker`, `EngineWorkerIsBuiltin`, the `engine/worker-compose.yaml` example), `SANDBOX_CONFIG_MOVED` and the `compose::add iii-sandbox` rewrite (D9).

Router messages (the `message` passed to `error_payload`):
- S303 (spec, verbatim): `no sandbox adapter configured: this engine is not managed by a Compose file; set sandbox.adapter in its engine config (Compose-managed engines default to sandbox-builtin)`
- S301 (spec, verbatim template): `adapter {adapter} not connected in namespace {namespace}; compose::add iii-sandbox in that project installs it`
- S302 (registry): `adapter {adapter} implements sandbox contract {major}.{minor}, but this engine needs major {CONTRACT_MAJOR}; install a {adapter} release whose major version is {CONTRACT_MAJOR}`
- S304 (registry, adapter-side): `{function_id} is not supported by adapter {adapter}`

## 5. Compose API (P2b produces; P5 relies on the behaviour)

```rust
// crates/iii-compose/src/sandbox.rs, `pub mod sandbox;` in lib.rs (spec)
pub enum Need { VmBundle(String), VmLocal(String), Explicit }   // (spec; no Added, D11)
pub fn needs(file: &ComposeFile) -> Vec<Need>;      // (spec) after lockfile::attach
pub fn wire(file: &mut ComposeFile, adapter_key: &str) -> Result<()>;          // (spec)
pub fn state_key(state_dir: &Path, key: &str) -> String;  // (registry) hex SHA-256(state_dir bytes, 0x00, key); state_dir = StateStore::dir()
pub struct RemoteVm { pub record: VmRecord /* status watch and poll task private */ }            // (spec)
impl RemoteVm { pub fn poll(&self) -> Outcome; pub async fn wait(&self) -> ExitStatus;
                pub async fn stop(&self, grace: Duration) -> ExitStatus; }   // as Supervised, process/unix.rs:194-244
// process/mod.rs (:71-75); lifecycle.rs:149 becomes `pub type Children = BTreeMap<String, Child>;`
pub enum Outcome { Exited(ExitStatus), Running, Unknown }                       // (spec)
pub enum Child { Process(Supervised), Vm(RemoteVm) }                            // (spec) same poll, wait, stop
// state.rs: ChildRecord (:82-92) keeps status, started_at, last_error, and changes
#[serde(default)] pub pid: Option<u32>,  #[serde(default)] pub birth: Option<BirthIdentity>,     // (spec)
#[serde(default, skip_serializing_if = "Option::is_none")] pub vm: Option<VmRecord>,             // (spec)
pub struct VmRecord { pub adapter: String, pub namespace: String, pub vm_id: String,
                      pub instance: u64 }   // (registry name; fields spec 7.4; no log_cursor, D11)
// DaemonState (:111-122) gains (registry): uuid v4 simple, in the 0600 state
#[serde(default, skip_serializing_if = "Option::is_none")] pub sandbox_token: Option<String>,
// config.rs: RawEngineSpec (:1017-1027) and EngineSpec (:328-333) gain `sandbox: Option<SandboxSpec>` (spec)
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)] #[serde(deny_unknown_fields)]
pub struct SandboxSpec { #[serde(default = "default_adapter")] pub adapter: String,   // DEFAULT_ADAPTER
                         #[serde(default)] pub namespace: Option<String> }           // (registry name; keys spec 6.3)
pub const CONFIGURABLE_ENGINE_WORKERS: &[&str] =
    &["configuration", "iii-worker-manager", "iii-http-functions", "iii-stream"];    // (spec 6.4)
```
- `EngineSpec.sandbox == None` means no section. EngineSpec equality already drives `EngineRestartRequired` (`daemon.rs:198-225`). The materialized `engine-config.yaml` (`managed_engine.rs:243-325`) always gets `sandbox: {adapter, namespace}`: the section's values, or `DEFAULT_ADAPTER` and the owner namespace (spec S1). Compose adds no adapter-name check of its own; the engine's `INVALID_SANDBOX_CONFIG` is the gate (registry).
- Generated container: `edit::upsert_container` writes it, so the marker is `edit::MARKER` = `"# added by compose::add"` (`edit.rs:115`, spec); no new constant. Version range `format!("^{CONTRACT_MAJOR}")` (spec S2). The reason (`sandbox adapter for: <key>`) is printed, never written (spec). `compose::add worker=iii-sandbox` is rewritten to this request in `add_configured_scoped`, before planning; its reply's `details.container` is the adapter key, `sandbox-builtin` by default (D11; P5's harness checks it). Attached projects use `DEFAULT_ADAPTER` for VM needs (D11).
- Adapter container env, set on that container only and added to `RESERVED_ENV` (`spawn.rs:49-57`): `III_SANDBOX_WORKER_TOKEN` = `DaemonState.sandbox_token` (spec 7.8); `III_SANDBOX_HOST_ROOTS` = `std::env::join_paths([<compose dir>, StateStore::package_cache()])` (registry), only these two roots (D11).
- CI: `ci.yml` job `compose-sandbox-e2e-linux` runs the spec 11 acceptance e2e and the router `sandbox::exec` smoke, pinning `sandbox-builtin` and the registry bundle `onboarding` at fixed versions (D5); P3 keeps it.
- Start check: `adapter::info`; major differs -> `SpawnFailed { container: <vm key>, message: <S302 text> }` (registry); name differs -> `SandboxAdapterMismatch`. `function_not_found` retried for 10 s (spec). `worker::prepare` keeps the 20-minute budget (`lifecycle.rs:1281`).
- Supervision: `worker::status` polled every 1 s; bus error -> `Outcome::Unknown`, which never restarts or cascades; S504 -> an exit with no code (spec 7.4); S503 on stop -> already stopped (registry). A VM exit becomes `ExitStatus::from_raw` (spec).

| Variant in `crates/iii-compose/src/error.rs` | `code()` | Text |
|---|---|---|
| `SandboxConfigMoved { replacement: String }` (registry) | `SANDBOX_CONFIG_MOVED` (spec) | spec 6.4 text verbatim, a blank line, then `replacement`: the generated container (marker, `worker: package://sandbox-builtin`, `version: '^1'`) with the old block under `config_override`. `'^1'`, not an exact version, because `validate_engine` (`config.rs:473`) is offline (registry) |
| `SandboxAdapterMismatch { container, expected, found }` (registry) | `SANDBOX_ADAPTER_MISMATCH` (spec) | `container '{container}' is the sandbox adapter '{expected}', but adapter::info names '{found}'. Set engine.sandbox.adapter to '{found}' or install '{expected}'` |
| `SandboxAdapterNotDeclared { adapter, needed_by }` (registry) | `SANDBOX_ADAPTER_NOT_DECLARED` (spec) | `{needed_by} needs the sandbox adapter '{adapter}', and --frozen does not edit the file. Run up once without --frozen, or: iii trigger compose::add worker=iii-sandbox` |
| existing `UnsupportedPlatform` (`error.rs:278`) | `UNSUPPORTED_PLATFORM` (spec) | `container` = the adapter key; also the registry's `/resolve` `unsupported_platform` refusal with no fallback (D11) |
| none: `SANDBOX_ADAPTER_IS_A_VM` is not in v1 (spec 2) | `DEPENDENCY_CYCLE` | an adapter that is a VM container is a cycle |
| `BundleNeedsAVm` (`error.rs:272`) | unchanged | kept as the Windows guard (D11) |
| `UnsupportedEngineWorker` | unchanged | text drops `iii-sandbox` and points to `engine.sandbox` (spec 6.4) |
| `EngineWorkerIsBuiltin` (`daemon.rs:2178-2191`) | unchanged | text unchanged: it names no worker list, and the `compose::add iii-sandbox` rewrite runs before it (P2b) |

No `compose::migrate` request, response or CLI form (spec 7.9).

## 6. Adapter-side names (P1 produces; P2b and P5 rely on them)

| Name | Value |
|---|---|
| Folder, manifest `name`, Cargo package, binary, function prefix | `sandbox-builtin` (spec) |
| `sandbox-builtin/iii.worker.yaml` | `language: rust`, `deploy: binary`, `manifest: Cargo.toml`, `bin: sandbox-builtin`; `config:` = today's `sandbox_daemon/iii.worker.yaml` config, with no `worker_vm` keys: resources come from the worker manifest within today's caps (D3). No skill (D8) |
| SDK identity | `WorkerMetadata.name = "sandbox-builtin"` (today `"iii-sandbox"`, `sandbox_daemon/mod.rs:131`); `III_WORKER_NAME` overrides it (`sdk/packages/rust/iii/src/iii.rs:656`) (registry) |
| Config | one `iii_config_client::fetch(&iii, <III_CONFIG_NAME, else "sandbox-builtin">)` at startup (`crates/config-client/src/lib.rs:93`); an error exits non-zero, no entry means serde defaults; no hot apply (spec 7.6; fallback registry) |
| `III_VM_*` tuning | stays the four env vars read at `cli/worker_manager/libkrun.rs:44-65` (registry; spec 7.6 lists them as keys: open question) |
| Guards | `III_SANDBOX_WORKER_TOKEN` (spec): absent or mismatched -> every `worker::*` answers S505. `III_SANDBOX_HOST_ROOTS` (registry): `host_path` must canonicalize under one root, else S501 naming the roots (D11) |
| State | `~/.iii/sandbox/workers/<vm_id>/` (spec), `vm_id` = first 32 hex of SHA-256(`state_key`); P2b's e2e checks it is gone after `compose::remove`. Its contents are P1's (D14). Sandbox and image caches keep today's paths |
| Guest env, `__vm-boot` | `__vm-boot --env` then the kernel cmdline, as today; no `/opt/iii` env file in v1 (spec 8.3). `__vm-boot` stays hidden with today's `VmBootArgs` (`cli/vm_boot.rs:16-130`), re-executing `current_exe()` |
| `.deploy/workers.yaml` | under `artifact:`, `features: [embed-init, embed-libkrunfw]`, `prebuild: [scripts/build-init.sh]`, `macos_entitlements: entitlements.plist` (spec), allowed at `.github/scripts/deployment_compiler.py:265` |
| CI | `ci.yml` jobs `sandbox-builtin-macos` and `sandbox-builtin-vm-linux` (spec); vm-linux runs the VM tests, `tests/worker_vm_e2e.rs` and the isolation e2e; making it required is a branch-protection step the plan index lists (D5). Workflow `sandbox-builtin-images.yml`, with only a `workflow_dispatch` trigger; P3 deletes iii's `docker-worker-images.yml` (D7) |
| Isolation e2e | `sandbox-builtin/scripts/sandbox-isolation-e2e.sh` through `iii trigger`; env `III_BIN`, `ADAPTER_BIN` (replaces `WORKER_BIN`); prefix in `SANDBOX_ID_PREFIX`, default `sandbox-builtin`, which P5 sets to `sandbox` with an explicit `sandbox:` binding in the script's own engine config, not under Compose (D13) (registry) |

`cli/*` paths are under `crates/iii-worker/src/` in the iii repo and move to `sandbox-builtin/src/cli/`.

## 7. Test doubles

| Double | Prefix | Language, home, registration | Built by |
|---|---|---|---|
| Fake adapter | `fake` (spec 10) | Rust, `engine/tests/sandbox_router_e2e.rs` (registry). A raw `tokio_tungstenite` worker, as in `engine/tests/namespace_conflict_e2e.rs`, in namespace `fake-ns`, registering `fake::adapter::info` (`AdapterInfo { adapter: "fake", contract: CURRENT }`, or major 2 for S302), `fake::exec` (echoes input and metadata), `fake::fs::ls` (returns an S211 payload) and `fake::create` (blocks until the socket drops, for `invocation_stopped`). Unit tests in `router.rs` may register `fake::*` in-process with `register_function_ns` | P2a |
| Sandbox mock | `sandbox-mock` (spec) | TypeScript, `ide/tests/e2e/workers/harness/src/cases-fs-sandbox.ts:69-97`, where today's `sandbox::fs::*` mocks live. The harness worker registers `sandbox-mock::fs::*` and `sandbox-mock::adapter::info` (`{adapter: "sandbox-mock", adapter_version: "0.0.0", contract: {major: 1, minor: 0}}`) in its own namespace `ide-e2e`; `ide/tests/e2e/config.yaml` gets top-level `sandbox: {adapter: sandbox-mock, namespace: ide-e2e}` (D13) | P5 |

## 8. Ownership

| Name | Producer | Consumers |
|---|---|---|
| `CONTRACT_MAJOR/MINOR`, `ContractVersion`, `PUBLIC_PREFIX`, `DEFAULT_ADAPTER`, `adapter_function_id`, `is_valid_adapter_name`, `OPERATIONS`, `WORKER_*` | P0 | P2a, P2b; P1 copies |
| Ephemeral types, `StreamChannelRef`; worker-VM types, `LogRecord` | P0 | P2a (schemas), P2b; P1 copies the worker-VM types |
| `SandboxErrorCode`, `error_payload`, `code_of`, `DOCS_BASE`, both fixtures, docs anchors | P0 | P2a, P2b; P3 (keeps the anchors); P1 copies the codes and both fixtures |
| `AdapterInfo`, `AdapterInfoRequest`, `ADAPTER_INFO_SUFFIX` | P0 | P2a, P2b; P1 copies; P5's mock writes the same JSON |
| `RouterInfo`, `ROUTER_INFO_ID` | P0 | P2a, P5 (code-runner probe checks `info != null`; harness) |
| `SandboxRouter`, `SandboxRouterConfig`, `EngineConfig.sandbox`, `normalize_worker_entries`, `ENGINE_RESERVED_PREFIXES`, `INVALID_SANDBOX_CONFIG`, dropping `iii-sandbox` from `CONFIGURABLE_ENGINE_WORKERS` | P2a | P2b, P3 (docs), P5 |
| `sandbox::{needs, Need, wire, state_key, RemoteVm}`, `Child`, `Outcome::Unknown`, `VmRecord`, `ChildRecord.vm`, `sandbox_token`, `SandboxSpec`, Compose errors, adapter env, the `compose::add iii-sandbox` reply, `compose-sandbox-e2e-linux` | P2b | P3 (keeps the job; docs), P5 (behaviour) |
| `sandbox-builtin` package, ids, config keys, state layout, env names, CI jobs, `SANDBOX_ID_PREFIX`, `ADAPTER_BIN` | P1 | P2b, P3 (docs), P5 |
| Fake adapter / `sandbox-mock` | P2a / P5 | none |
