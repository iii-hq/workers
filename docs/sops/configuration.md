# Integrating a worker with the `configuration` worker

How to move a worker's runtime config out of a static `config.yaml` and onto
the built-in **`configuration`** worker: a schema-validated, reactive registry
that other workers and the operator console can read, validate, and edit on a
live bus.

This is the **advanced** alternative to the baseline static-config pattern in
[`binary-worker.md`](binary-worker.md) §5. Reach for it when config must be
observable, hot-reloadable, or shared. Reference implementations:
`session-manager`, `context-manager`, `shell`, `storage`, `database`, `coder`.

## 1. What the `configuration` worker is

A server-side registry of named entries. Every entry has an id (e.g.
`session-manager`, `llm-router`), a human-readable name and description, a JSON
Schema describing the value shape, and a JSON value validated against that
schema. Workers call `configuration::register` once at startup to declare their
schema and `configuration::set` to publish values; consumers call
`configuration::get` / `configuration::list` to read, and bind a
`configuration` trigger to react to changes without polling.

The default `fs` adapter persists one YAML file per id under
`./data/configuration` and watches the directory, so manual edits surface as
`configuration:updated` events the same way SDK calls do. The worker is enabled
by default in the engine.

### When to use

- A worker is migrating its block out of a static `config.yaml` and wants a
  typed, observable surface other workers can read and validate against.
- Two workers need to agree on the same values without one polling the other.
- An operator should edit one place (a YAML file or the console) and have the
  change propagate to every subscriber without a restart.

### Boundaries

- Not a general-purpose key/value store — every entry needs a registered JSON
  Schema. Use `iii-state` for free-form values.
- No partial updates: `set` always replaces the whole value. Build the new
  value client-side and ship it in one call.
- Schemas are not version-checked across re-registrations — re-registering with
  an incompatible schema replaces it. Coordinate migrations out-of-band.

## 2. Function surface

All ids are kebab-case (`<worker>::<verb>`), per [`binary-worker.md`](binary-worker.md) §7:

- `configuration::register` — declare an id with name, description, JSON
  Schema, and an optional `initial_value`; idempotent re-registration replaces
  the schema/metadata but preserves any stored value.
- `configuration::set` — replace the value for a registered id; validates
  against the schema and emits `configuration:updated`.
- `configuration::get` — read one entry by id; expands `${VAR:default}`
  against live env unless `raw: true`.
- `configuration::list` — enumerate registered ids with name/description/schema
  (never the value).
- `configuration::schema` — read schema/name/description for one id.

`register` and `set` are the only mutators; reads are cache-backed and expand
`${VAR:default}` against the live process env on every call, so env changes
propagate without a restart.

## 3. The integration recipe (Rust binary)

### a. `src/config.rs` — make the config schema-able and splittable

Keep the `WorkerConfig` struct + `serde(default)` + `default_*()` +
`impl Default` from [`binary-worker.md`](binary-worker.md) §5, then add:

- Derive **`Serialize` + `JsonSchema`** (alongside `Deserialize`); keep
  `#[serde(deny_unknown_fields)]`.
- `from_yaml(&str)` / `from_file(&str)` — env-expand `${NAME}` against the
  process env, then parse. This is the SEED path only.
- `from_json(&Value)` — parse a value already env-expanded by the worker (do
  **not** re-expand).
- `to_json(&self) -> Value` and `json_schema() -> Value` (a
  `schemars::schema_for!` with the shipped defaults attached as `example`).
- `boot_signature(&self) -> BootSignature` — the fields consumed **once at
  boot** (adapters built then and never rebuilt). Everything else is a per-call
  tuning knob that can hot-reload. If every field is boot-time, the signature is
  the whole config and all changes are restart-required.

### b. `src/configuration.rs` — the integration module

Mirror [`context-manager/src/configuration.rs`](../../context-manager/src/configuration.rs).
Provide:

- `pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>` — the hot-swappable
  snapshot shared with handlers.
- `CONFIG_ID = "<worker>"`, `CONFIG_FN_ID = "<worker>::on-config-change"`, and
  retry/backoff constants.
- `register_config(iii, seed)` — register `json_schema()`; install `seed` as
  `initial_value` when present, else seed the built-in default only when no
  value is stored yet (safe to call every boot).
- `fetch_config(iii)` — read the authoritative, env-expanded value
  (`NOT_FOUND` ⇒ built-in default).
- `apply_config(cell, cfg)` / `reloadable(cfg, boot_sig)` — swap the snapshot,
  refusing any change to the boot signature (restart required).
- `register_config_trigger(iii, cell, boot_sig)` — register the
  `<worker>::on-config-change` handler and bind a `configuration` trigger
  filtered to this id (see §4). The handler **re-fetches** via
  `configuration::get` and ignores the trigger payload, so a direct call can
  never inject config.

### c. `src/main.rs` — boot order

```text
1. parse CLI (--config is now only a SEED)
2. connect to the engine
3. register_config(seed)  +  fetch_config()        # required boot dependency
4. resolve adapters from the fetched config (capture boot_signature first)
5. build the ConfigCell; register functions
6. register_config_trigger(cell, boot_sig)         # LAST — closes over the cell
```

`configuration` is a **required boot dependency**: a failed register/fetch
aborts startup. Build the `ConfigCell` once and share it between the service
and the trigger so a live `set` of a tuning knob is picked up per call. Bind the
trigger **last** so its handler closes over the fully-built cell.

### d. `config.yaml` — now a SEED

Add a header making clear it is not the source of truth: it only populates
`initial_value` on the first `configuration::register` (when nothing is stored
for the id). After that the stored value is authoritative; edit it with
`configuration::set id=<worker>` or by editing the persisted file. Mirror
[`coder/config.yaml`](../../coder/config.yaml).

### e. `iii-permissions.yaml` — deny the reload hook

`<worker>::on-config-change` must never be agent-callable; add
`'!<worker>::on-config-change'` next to the existing
`'!storage::on-config-change'` / `'!database::on-config-change'` denies. It is
defense-in-depth: the handler already re-fetches from `configuration::get`, so a
direct call cannot inject config.

## 4. Reactive triggers

Bind a `configuration` trigger when a function should run on every
register/set/delete — including external `fs` edits and bridge-forwarded events:

```rust
iii.register_trigger(RegisterTriggerInput {
    trigger_type: "configuration".to_string(),
    function_id: "<worker>::on-config-change".to_string(),
    config: json!({
        "configuration_id": "<worker>",              // omit to receive every id
        "event_types": ["configuration:updated"],    // subset of registered|updated|deleted
    }),
    metadata: None,
})?;
```

Reads never fire triggers. If you only need the new value inside the same
function that wrote it, `configuration::set` already returns
`old_value` / `new_value` — bind a trigger only when a *different* component
should react.

## 5. Hot-reload vs restart-required

The **boot signature** (§3a) is the contract: on `configuration:updated`,
`on-config-change` re-fetches and, when the boot signature is unchanged, swaps
the snapshot so handlers read new tuning knobs per call. A boot-signature change
is **refused** (logged "restart required", the previous snapshot kept) — those
adapters are built once at boot. Always keep the previous snapshot on any
failure path; never serve a half-applied config.

## 6. Checklist

- [ ] `WorkerConfig` derives `Serialize` + `JsonSchema`; has
      `from_yaml`/`from_file`/`from_json`/`to_json`/`json_schema`/`boot_signature`.
- [ ] `src/configuration.rs` mirrors the reference: register / fetch / trigger /
      reloadable + a `ConfigCell`.
- [ ] `main.rs`: connect → register+fetch (fatal on failure) → build adapters →
      register functions → bind the `configuration` trigger last.
- [ ] `config.yaml` carries a SEED header; it is no longer the source of truth.
- [ ] `'!<worker>::on-config-change'` denied in `iii-permissions.yaml`.
- [ ] README "Configuration" section documents the id, the hot-reload vs
      restart-required split, and the seed semantics.
- [ ] Unit tests cover `boot_signature` (tuning-only vs restart-required) and a
      JSON round-trip; they run engine-free in CI.
