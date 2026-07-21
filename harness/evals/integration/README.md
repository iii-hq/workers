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

# Validate all five fixtures, including quarantined reproductions.
make -C harness integration-validate

# Run one scenario directly.
harness-integration run \
  --engine-bin <iii> \
  --harness-bin <harness> \
  --worker-bin queue=<queue> \
  --worker-bin session-manager=<session-manager> \
  --worker-bin context-manager=<context-manager> \
  --worker-bin iii-directory=<iii-directory> \
  --scenario C-E2E-001 \
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

Each scenario is one Rust builder module: `src/scenarios/<slug>.rs`, a
function that builds the authored data through the typed builders in
`src/scenarios/builder.rs` and registers it in `src/scenarios/mod.rs`. There
is no YAML layer — the authored shape is enforced by the type system at
`cargo build` and is never serialized. Model/provider, session id,
idempotency key, native function policy, run-scoped function ids, request
matchers, response frames, common completion checks, and system prompt hash
are inferred by the compiler.

A typical authored function scenario is:

```rust
// src/scenarios/my_function_case.rs
pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new("C-E2E-010", "The allowed function runs once.")
        .send(Send::message("Call the recorder once."))
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
        .generation(Reply::function_call("record", json!({ "value": "expected" })))
        .generation(Reply::text("recorded once"))
        .expect(
            Expect::new()
                .assistant_text("recorded once")
                .call(TargetCall::counted("record", 1).payload(json!({ "value": "expected" }))),
        )
}
```

Add the module and its slug to the list in `src/scenarios/mod.rs`, then:

```bash
cargo test                                     # builder, snapshot, and contract tests
REGEN_SCENARIO_SNAPSHOTS=1 cargo test --test scenario_compilation
harness-integration validate --scenario all
harness-integration render C-E2E-010
```

Builders produce data only — a builder that derives scenario content from
control flow is rejected in review. The compiled snapshot under
`tests/snapshots/<slug>.compiled.json` is the review artifact; commit the
regenerated snapshot with the new module. New scenarios are runnable by
default; chain `.quarantine()` only for a known reproduction that should be
excluded from `run --scenario all`. `render` prints deterministic canonical
JSON with the complete compiled request, router script, expectations, and
system prompt.

`Function::recorder()` is the canonical string-in/`recorded`-out fixture;
`Function::new(...)` builds any other controlled function and `.hidden()`
marks a hook-only one. Function aliases become `<run_id>::<alias>`.
`Send::message(...).allow([...])` can narrow the exposed aliases or be an
empty list to disable dispatch. `Release::execute()` and `Release::deliver()`
name the held-call action. Typed text and function-call replies cover normal
cases; `.recovery_boundary()` grades a reply against the durable outcome only,
where a fault restart or hook release may rebuild the request. `.match_overrides(...)`
and `RouterReplyV1::Raw` remain the deeper escape hatches for unusual wire
contracts.

Timeout defaults are 60 seconds for readiness, 60 seconds for the scenario,
and 15 seconds for teardown. Positive values can be overridden with the
`*_timeout_ms` builders; one readiness budget is shared by the full probe/arm
sequence.

## Checked-in scenarios

| id | slug | status |
|---|---|---|
| C-E2E-001 | `streamed-text` | streamed text reaches durable completion |
| C-E2E-002 | `exactly-once-function` | a native function executes exactly once |
| C-E2E-505 | `hold-mutation-505` | quarantined reproduction for issue #505 |
| C-E2E-506 | `hook-held-release-506` | quarantined reproduction for issue #506 |
| C-E2E-507 | `crash-recovery-507` | quarantined reproduction for issue #507 |

`run --scenario all` excludes quarantined scenarios. An explicit id or slug
runs it; `validate --scenario all` always includes it.

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
- Router and grader comparisons use explicit JSON array policies.

Each run writes `result.json`, `execution.json`, `teardown.json`, and
`stack.json` below `target/integration/<run-id>/`; scenario evidence lives
under `scenarios/<scenario-id>/`. `result.json` contains the stable
byte-comparable verdict. `execution.json` contains the run id, timing,
scenario id, and SHA-256 of the exact `result.json` bytes. Passing runs retain
the compact reports and remove heavyweight stack state unless
`--retain-success` is supplied.

The shared `scenarios/system-prompt.txt` is the single prompt golden. The
compiler appends the inferred session and function policy, then hashes the
result for strict router matching.
