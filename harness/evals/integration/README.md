# Harness integration E2E

Deterministic public-path regression tests for the harness. Every scenario
boots a fresh isolated stack with the pinned engine and real queue,
session-manager, context-manager, iii-directory, and harness workers. Only
the `router::*` model boundary is replaced by a strict scripted worker.

No provider key or network access is required.

## Run

```bash
# Build the stack and run every non-quarantined scenario.
make -C harness integration-e2e III_BIN=<path-to-iii>

# Validate every fixture, including quarantined reproductions.
make -C harness integration-validate

# Run one scenario directly.
harness-integration run \
  --engine-bin <iii> \
  --harness-bin <harness> \
  --worker-bin queue=<queue> \
  --worker-bin session-manager=<session-manager> \
  --worker-bin context-manager=<context-manager> \
  --worker-bin iii-directory=<iii-directory> \
  --scenario E2E-001 \
  --repeat 2
```

The engine is never downloaded by the runner. CI builds the source revision
in `engine.lock`; local runs receive the corresponding binary through
`III_BIN` or `--engine-bin`.

Exit codes are:

- `0`: every selected scenario passed;
- `2`: contract failure or scenario timeout;
- `3`: setup, process, or runner error.

`--repeat N` boots a fresh stack for every repetition and requires the
byte-stable result contract to be identical. A mismatch is a runner error.

## Create a scenario

Each scenario is one Rust module: `src/scenarios/<slug>.rs`, one `scenario()`
function that builds the authored stimulus through the typed builders in
`src/scenarios/builder.rs` and closes the chain with `.verify(|run| ...)`,
registered in `src/scenarios/mod.rs`. A scenario without checks does not
typecheck. There is no YAML layer — the authored
shape is enforced by the type system at `cargo build` and is never
serialized. Model/provider, session id, idempotency key, native function
policy, run-scoped function ids, request matchers, response frames, and
system prompt hash are inferred by the compiler.

A typical authored function scenario is:

```rust
// src/scenarios/my_function_case.rs
pub(super) fn scenario() -> Scenario {
    AuthoredScenario::new("E2E-010", "The allowed function runs once.")
        .trigger(Harness::send("Call the recorder once."))
        .function(
            "record",
            Function::new(
                "Record one value.",
                json!({
                    "type": "object",
                    "additionalProperties": false,
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"]
                }),
                json!({
                    "content": [{ "type": "text", "text": "recorded" }],
                    "is_error": false
                }),
            ),
        )
        .model((
            Reply::function_call("record", json!({ "value": "expected" })),
            Reply::text("recorded once"),
        ))
        .verify(|run| {
            let calls = run.calls("record");
            anyhow::ensure!(calls.len() == 1, "record ran {} times", calls.len());
            anyhow::ensure!(calls[0].payload == json!({ "value": "expected" }));
            anyhow::ensure!(!run.has_duplicate_messages());
            Ok(())
        })
}
```

The scenario returns a dataset; the author writes the checks in plain Rust.
The floor is enforced by the runner before `verify` is called — turn
completed (terminal status, lifecycle delivered exactly once), script fully
consumed (every scripted generation used, none extra), and a clean send —
and any violation is a `contract_failure` with a `floor: ` message.
`verify(run)` then receives the full `RunEvidence` dataset (send response,
final status, transcript, recorder events, router consumption) for
scenario-specific checks; accessors such as `assistant_texts()`,
`message_counts()`, `calls(alias)`, and `all_calls_closed()` cover the
recurring reads. The runner catches panics, so `assert!`/`assert_eq!` are
allowed; prefer `anyhow::ensure!` where a message helps.

Add the module and its slug to the list in `src/scenarios/mod.rs`, then:

```bash
cargo test                                     # builder and contract tests
harness-integration validate --scenario all
harness-integration render E2E-010
```

Builders produce data only — a builder that derives scenario content from
control flow is rejected in review. New scenarios are runnable by
default; chain `.quarantine()` only for a known reproduction that should be
excluded from `run --scenario all`. `render` prints deterministic canonical
JSON with the complete compiled request, router script, and system prompt.

`Function::recorder()` is the canonical string-in/`recorded`-out fixture;
`Function::new(...)` builds any other controlled function and `.hidden()`
marks a hook-only one. Function aliases become `<run_id>::<alias>`; every
exposed function is dispatchable. `Release::execute()` releases a held call
for execution. Typed text and function-call replies cover normal cases;
`.recovery_boundary()` matches a reply against the durable outcome only,
where a fault restart or hook release may rebuild the request, and
`.match_overrides(...)` is the remaining escape hatch for intentionally
different wire shapes (the Console's agent-trigger policy).

Timeout defaults are 60 seconds for readiness, 60 seconds for the scenario,
and 15 seconds for teardown. Positive values can be overridden with the
`*_timeout_ms` builders; one readiness budget is shared by the full probe/arm
sequence.

## Checked-in scenarios

| id | slug | status |
|---|---|---|
| E2E-001 | `streamed-text` | streamed text reaches durable completion |
| E2E-002 | `exactly-once-function` | a native function executes exactly once |
| UI-001 | `console-streamed-text` | the production Console sends and renders streamed text |
| E2E-505 | `hold-mutation-505` | quarantined reproduction for issue #505 |
| E2E-506 | `hook-held-release-506` | quarantined reproduction for issue #506 |
| E2E-507 | `crash-recovery-507` | quarantined reproduction for issue #507 |

`run --scenario all` includes non-quarantined direct scenarios. Console-driven
scenarios run through `serve --scenario <id-or-slug>` and Playwright. An
explicit quarantined direct scenario still runs; `validate --scenario all`
always includes every driver and quarantine state.

## Runtime and evidence

The lifecycle is allocate → boot → probe → arm → send → optional fault or
release → await → collect → grade → teardown → report.

- Readiness inspects structured function, trigger, queue, and configuration
  surfaces.
- All RPCs and polling share monotonic phase deadlines.
- The recorder keeps configuration and snapshots in process; only controlled
  target functions and the lifecycle sink are registered with the engine.
- Recorder acknowledgements happen only after append and `fsync`.
- Child processes run in dedicated process groups and teardown signals the
  complete group and direct child with SIGTERM followed by SIGKILL, within
  one hard cleanup budget.
- Router and evidence comparisons use explicit JSON array policies.

Each run writes `result.json`, `execution.json`, `teardown.json`, and
`stack.json` below `target/integration/<run-id>/`; scenario evidence lives
under `scenarios/<scenario-id>/` (transcript, status, router calls, target
calls, lifecycle events). `result.json` contains the stable byte-comparable
verdict: the classification plus the first floor or verify failure message,
with run/session/turn ids scrubbed to placeholders. `execution.json` contains
the run id, timing, scenario id, and SHA-256 of the exact `result.json`
bytes. In serve mode, `serve-result.json` additionally carries the raw
serialized `RunEvidence` (real ids) so Playwright can check it against the
ready manifest. Passing runs retain the compact reports and remove
heavyweight stack state unless `--retain-success` is supplied.

The compiler uses the Harness's embedded `prompts/default.txt` directly,
appends the inferred session and function policy, then hashes the result for
strict router matching. A scenario may explicitly override the router's prompt
matcher in its builder without replacing the shared prompt source.
