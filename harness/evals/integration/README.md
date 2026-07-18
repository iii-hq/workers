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
  --scenario C-E2E-001
```

The engine is never downloaded by the runner. CI builds the source revision
in `engine.lock`; local runs receive the corresponding binary through
`III_BIN` or `--engine-bin`.

Exit codes are:

- `0`: every selected scenario passed;
- `2`: contract failure or scenario timeout;
- `3`: setup, process, or runner error.

## Create a scenario

Each scenario is one file: `scenarios/<slug>/scenario.yaml`. Model/provider,
session id, idempotency key, native function policy, run-scoped function ids,
request matchers, response frames, common completion checks, and system
prompt hash are inferred.

```bash
harness-integration init \
  --id C-E2E-010 \
  --name my-function-case \
  --description "The allowed function runs once." \
  --kind function

harness-integration validate --scenario my-function-case
harness-integration render C-E2E-010
```

`init` supports `text`, `function`, `hook`, and `crash` templates and refuses
to overwrite an existing directory or reuse an existing scenario id. New
templates are runnable by default; set `quarantine: true` only for a known
reproduction that should be excluded from `run --scenario all`. `render`
prints deterministic canonical JSON with the complete compiled request,
router script, expectations, and system prompt.

A typical authored function scenario is:

```yaml
schema_version: "1"
id: C-E2E-010
description: The allowed function runs once.

send:
  message: Call the recorder once.

functions:
  record:
    description: Record one value.
    request_schema:
      type: object
      additionalProperties: false
      properties:
        value: { type: string }
      required: [value]
    response:
      content:
        - { type: text, text: recorded }
      is_error: false

router:
  generations:
    - reply:
        type: function_call
        function: record
        arguments: { value: expected }
    - reply:
        type: text
        text: recorded once

expect:
  assistant_text: recorded once
  calls:
    - function: record
      count: 1
      payload: { value: expected }
```

Function aliases become `<run_id>::<alias>`. Set `expose: false` for
hook-only functions. `send.allow` can narrow the exposed aliases or be an
empty list to disable dispatch. Typed text and function-call replies cover
normal cases; `match_overrides` and `type: raw` remain escape hatches for
recovery boundaries and unusual wire contracts.

Timeout defaults are 60 seconds for readiness, 60 seconds for the scenario,
and 15 seconds for teardown. Positive values can be overridden under
`timeouts`; one readiness budget is shared by the full probe/arm sequence.

## Checked-in scenarios

| id | directory | status |
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

Each run writes detailed evidence below
`target/integration/<run-id>/scenarios/<scenario-id>/`. `result.json` contains
the stable byte-comparable verdict; `execution.json` contains the run id,
timestamps, and duration. Passing runs retain the compact reports and remove
heavyweight stack state unless `--retain-success` is supplied.

The shared `scenarios/system-prompt.txt` is the single prompt golden. The
compiler appends the inferred session and function policy, then hashes the
result for strict router matching.
