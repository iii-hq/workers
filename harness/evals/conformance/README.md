# Harness conformance E2E

Deterministic regression track for the harness. Each scenario boots a fresh
isolated iii stack (pinned engine + real queue, session-manager,
context-manager, iii-directory, harness), replaces only the `router::*`
model boundary with a strict scripted worker, and grades structured public
evidence (send response, status, full transcript, recorder log, lifecycle
events, process state) with pure-code invariants. No model key, no network.

The authoritative architecture spec (harness-evaluation tech spec) lives in
the **iii repo**; this README carries the runner's operational docs and the
implementation-verified corrections to that spec.

## Running

```bash
# Build + run every non-quarantined scenario against the pinned engine:
make -C harness conformance-e2e III_BIN=<path-to-iii-engine>

# Direct CLI (debug binaries, one scenario):
harness-conformance \
  --engine-bin <iii> --harness-bin <harness> \
  --worker-bin queue=<queue> --worker-bin session-manager=<sm> \
  --worker-bin context-manager=<cm> --worker-bin iii-directory=<dir> \
  --scenario C-E2E-001 --artifacts-dir target/conformance

# Validate fixtures without booting a stack:
harness-conformance --validate-only --scenario all
```

The engine is never downloaded: pass `--engine-bin`/`III_BIN` (CI builds the
revision pinned in `engine.lock`). Exit codes: 0 all pass, 2
contract_failure/timeout, 3 runner_error/setup_error/process_crash.

### Console recording (diagnostics only, never an oracle)

`--console-bin <console>` spawns the console worker per-run;
`--record-console` captures the chat view to
`scenarios/<id>/console-recording.webm` via headless system Chrome
(`CONFORMANCE_CHROME` overrides the binary; run `pnpm install` in
`tools/console-recorder` once beforehand — the runner never downloads
during a test). Send is held until the recorder page loads so short turns
are fully captured.

## Scenarios

| id | dir | status |
|---|---|---|
| C-E2E-001 | `scenarios/streamed-text` | green — streamed text reaches durable completion |
| C-E2E-002 | `scenarios/exactly-once-function` | green — allowed function executes exactly once |
| C-E2E-505 | `scenarios/hold-mutation-505` | quarantined — [iii-hq/workers#505](https://github.com/iii-hq/workers/issues/505) |
| C-E2E-506 | `scenarios/hook-held-release-506` | quarantined — [iii-hq/workers#506](https://github.com/iii-hq/workers/issues/506) |
| C-E2E-507 | `scenarios/crash-recovery-507` | quarantined — [iii-hq/workers#507](https://github.com/iii-hq/workers/issues/507) |

Quarantined scenarios reproduce a known-open defect: they assert the
EXPECTED behavior, fail until the fix lands, are excluded from
`--scenario all`, and run by explicit id. Unquarantine (delete the
`quarantine: true` line) when the fix merges — the repro becomes the
permanent regression gate.

## Invariant registry (v1)

`scenario.yaml` invariant ids implemented by the grader: `send.flags`,
`transcript.message_counts`, `transcript.assistant_text`,
`transcript.no_duplicates`, `transcript.function_result`,
`transcript.calls_closed`, `status.terminal`, `lifecycle.completed_once`,
`router.generations_consumed`, `target.calls` (with optional `payload` /
`payload_subset`). Unknown ids fail closed.

## Implementation notes (corrections to the original spec)

Verified against the running stack; these supersede the spec text where
they disagree.

1. **Expected system prompt is a template, hashed at Arm.** The harness
   always appends aid lines (`Your session id is <id>.`, a policy paragraph
   for narrowed allow-lists, a working-directory line unless
   `default_filesystem_root: "off"` is seeded — the runner seeds it). The
   `expected/system-prompt.txt` fixtures carry `{{session_id}}`/`{{run_id}}`
   placeholders; the runner pre-chooses the session id (sent as
   `send.session_id`), expands at Arm, and computes the matcher's sha256.
2. **The harness boots after Arm.** Native exposure reads the harness's
   boot-seeded, reactively-refreshed registry snapshot
   (`harness/src/discovery.rs`); the run-scoped recorder target must exist
   before the harness starts or the first turn races the refresh. Boot
   order: engine → queue → iii-directory → session-manager →
   context-manager → Probe → Arm → harness → probe harness surface →
   lifecycle/hook bindings (need harness-registered trigger types, held
   until visible in `engine::registered-triggers::list`) → Send.
3. **Configuration readiness is seeded-keys-subset, not byte-compare** —
   workers store their resolved config (seed merged with defaults).
4. **The engine stamps `_caller_worker_id` into trigger payloads** — strict
   wire handlers strip top-level `_`-prefixed members; the recorder strips
   them from evidence.
5. **`router::models::get` accepts a provider-less lookup** (llm-router
   resolves by id; the mandatory context-manager depends on it) — the
   scripted router mirrors that instead of the spec's exact-provider rule.
6. **`request_id` steps are 0-based**: the second generation matches
   `^t_[0-9a-f]{32}:1$`.
7. **Durable transcript entries do not persist `usage`,** and the
   model-visible replay of a streamed assistant message carries the seeded
   entry's `stop_reason` (`"end"`) — the turn loop updates only the entry's
   content. `transcript.assistant_text` asserts text only; usage is pinned
   on the wire by the script's frame/response contract.
8. **`recorder.json` is folded into `scenario.yaml`** (the scenario schema
   embeds `RecorderConfigV1`).
9. **C-E2E-001 declares a never-called target** (`{{run_id}}::unused`,
   `target.calls {count: 0}`) — the schema requires a target and the
   zero-count doubles as the forbidden-side-effect oracle.
10. **Barriers are schema-valid but rejected by the v1 runner** — the
    release mechanism is unspecified; no fixture uses them.
11. **Engine YAML facts** (pinned source): `iii-worker-manager` accepts
    `config: {host, port}`; mandatory builtins merge into a config-file
    boot but `enabled_by_default` builtins (`iii-state`, `iii-stream`,
    `iii-cron`) must be listed explicitly; the builtin `configuration`
    worker accepts `adapter: {name: fs, config: {directory}}`.
12. **Fault seeds** (`fault: {kind: engine_sigkill, after_target_calls,
    restart_delay_ms}`) SIGKILL and respawn the engine mid-call; the
    recorder target's `response_delay_ms` opens the injection window.
    Observed on engine 0.21.8-next.1: the in-flight `harness-turn` job
    strands in the surviving queue worker (30-minute visibility timeout),
    the turn record dies with the engine's in-memory `iii-state`, and the
    transcript keeps a dangling `function_call` (C-E2E-507).
13. **Hook-chain scenarios**: `recorder.extra_functions` hosts run-scoped
    hook implementations with declared decisions; scenario `bindings`
    create `harness::hook::*` trigger bindings after harness boot; scenario
    `release` resolves a held call via `harness::function::resolve` once it
    parks. C-E2E-505/506 use these to pin the hold/release argument
    semantics.

## Layout

```
Cargo.toml            # standalone workspace (repo convention)
engine.lock           # pinned engine source CI builds (never downloaded)
src/                  # runner: stack, readiness, scripted router, recorder,
                      # scenario loop, grader, artifacts, console recording
schemas/              # golden JSON Schemas (REGEN_SCHEMAS=1 cargo test --test schemas)
scenarios/<name>/     # scenario.yaml + router-script.json + expected/system-prompt.txt
tools/console-recorder/  # playwright-core recording sidecar (node)
target/conformance/<run_id>/  # per-run artifacts and evidence
```
