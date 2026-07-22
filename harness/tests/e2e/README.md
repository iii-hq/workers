# Harness integration E2E

Deterministic public-path regression tests for the harness. Each scenario
boots a fresh isolated stack with the pinned engine and the real queue,
session-manager, context-manager, iii-directory, and harness workers. Only
the `router::*` model boundary is replaced by a strict scripted worker.

No provider key or network access is required.

## Scenarios

| id | slug | driver | coverage |
|---|---|---|---|
| E2E-001 | `streamed-text` | direct | streamed text reaches durable completion |
| E2E-002 | `exactly-once-function` | direct | a native function executes exactly once |
| UI-001 | `console-streamed-text` | playground | a message sent by the Console streams to durable completion |
| UI-002 | `multi-turn-traces` | playground | a native function turn and a Console turn expose distinct traces and function-call events |

Each fixture is defined end to end in its own `src/scenarios/*.rs` file with a
small typed DSL. The scenario keeps its send policy, router request matchers,
response behavior, recorder configuration, function history, and verification
visible at the call site. Builders compile directly to the runtime types; there
is no YAML, macro layer, inferred history, or generic authored-scenario
compiler.

## Run the direct scenarios

```bash
make -C harness integration-e2e III_BIN=<path-to-pinned-iii>

# Select one direct scenario by id or slug.
make -C harness integration-e2e \
  III_BIN=<path-to-pinned-iii> \
  INTEGRATION_SCENARIO=E2E-001
```

The engine is never downloaded by the runner. CI builds the source revision
in `engine.lock`; local runs receive the corresponding binary through
`III_BIN` or `--engine-bin`.

Exit codes are:

- `0`: every selected scenario passed;
- `2`: contract failure or scenario timeout;
- `3`: setup, process, or runner error.

## Open an isolated Console playground

```bash
make -C harness integration-playground III_BIN=<path-to-pinned-iii>
```

The command builds and starts the production Console together with the
isolated integration stack. It creates the scenario session and prints its
Console URL. For the default UI-001 scenario:

1. Open the printed URL.
2. Select the pre-created integration session.
3. Send `Return the console fixture phrase.` through the message composer.
4. Wait for `console fixture complete`.
5. Stop the command with Ctrl-C.

After a completed turn, shutdown collects evidence, grades the scenario, and
writes `playground-result.json`. Stopping before a turn completes is a
contract failure.

The underlying command accepts one scenario only:

```bash
harness-integration playground \
  --engine-bin <iii> \
  --harness-bin <harness> \
  --console-bin <console> \
  --worker-bin queue=<queue> \
  --worker-bin session-manager=<session-manager> \
  --worker-bin context-manager=<context-manager> \
  --worker-bin iii-directory=<iii-directory> \
  --scenario console-streamed-text
```

`--ready-file <path>` optionally publishes an atomic JSON manifest for
Playwright. The manifest includes the engine and Console URLs, session,
scenario, model, message, controlled function ids, compiled direct send, and
result path. There is no separate start signal: Playwright either invokes the
compiled send through the SDK or submits through the Console UI.

## Validate fixtures

```bash
make -C harness integration-validate
cargo test --manifest-path harness/Cargo.toml -p harness-integration
cargo clippy --manifest-path harness/Cargo.toml \
  -p harness-integration --all-targets -- -D warnings
```

`validate --scenario all` checks exactly the four fixtures. `run --scenario
all` executes only E2E-001 and E2E-002; UI-001 and UI-002 must use
`playground`.

The fixture tests pin:

- the streamed frame sequence and terminal response agreement;
- function-call and function-result history for E2E-002;
- the Console-specific system-prompt and `agent_trigger` tool matchers;
- serialization round trips and the authoritative `harness::send` schema.

## Runtime and evidence

The direct lifecycle is allocate → boot → arm → send → await → collect →
grade → teardown → report. Playground replaces send with an externally
initiated Console or SDK turn and waits for shutdown after completion.

- Completion is driven by the `harness::turn-completed` lifecycle event.
- All RPCs and polling use bounded monotonic deadlines.
- Recorder acknowledgements happen only after append and `fsync`.
- Child processes run in dedicated process groups and teardown uses SIGTERM
  followed by SIGKILL within one hard cleanup budget.
- Router matching is explicit for all request fields.

Direct runs write `result.json`, `execution.json`, `teardown.json`, and
`stack.json` below `target/integration/<run-id>/`. Scenario evidence includes
the transcript, status, router calls, controlled target calls, and lifecycle
events. `result.json` is stable across runs because concrete run, session, and
turn ids are scrubbed from failure text; `execution.json` records the SHA-256
of those exact result bytes.

Playground runs use `target/console-e2e/<run-id>/` and additionally write
`playground-ready.json` and `playground-result.json`. Passing runs keep compact
reports and remove heavyweight stack state.
