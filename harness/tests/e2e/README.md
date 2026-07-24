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
| E2E-003 | `reseed-parked-message` | direct | a message parked during a turn's failing final step is delivered by a harness-reseeded turn |
| E2E-004 | `join-spec-mismatch` | direct | a join predecessor registered with a divergent reaction spec is rejected at registration |
| E2E-005 | `reaction-policy-inheritance` | direct | a reaction with no options inherits the registering turn's dispatch policy |
| E2E-006 | `state-worker-sidecar` | direct | a state-key reaction fires through the standalone state worker with its metadata sidecar (probe-hook driven) |
| E2E-007 | `coalesced-fire` | direct | a burst past the fire-rate cap coalesces: cap + 1 dispatches, the trailing one stamped `__coalesced_fires` (shrunken gate via harness env; probe-side whole-run call evidence) |
| E2E-008 | `reaction-unregisters-run` | direct | a reaction session in the registrant's lineage unregisters the registrant's subscription (serve-time capture of the runtime sub id; probe-side call await) |
| E2E-009 | `late-join-predecessor-replay` | direct | a join predecessor registered after its watched session completed receives a catch-up completion fire (level-triggered join barrier; `probe_after_calls` gating) |
| UI-001 | `console-streamed-text` | playground | a message sent by the Console streams to durable completion |
| UI-002 | `multi-turn-traces` | playground | a native function turn and a Console turn expose distinct traces and function-call events |

Each fixture is defined end to end in its own `src/scenarios/*.rs` file with a
small typed DSL. The scenario keeps its send policy, router request matchers,
response behavior, controlled function, function history, and verification
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

`validate --scenario all` checks every fixture. `run --scenario all` executes
the direct scenarios (E2E-001, E2E-002, E2E-003); UI-001 and UI-002 must use
`playground`. E2E-003 produces two terminal turns from one send: generation 1
steers a message into the running session (it parks durably) and then fails,
so the harness's failed finalize drains the parked row and reseeds a turn to
react to it. The failed route is deliberate — a park during a *completing*
terminal generation is always delivered earlier by the loop's steering check,
so only the failed finalize (which has no steering check) reaches the drain
deterministically from the public boundary; both finalize paths share the
drain-and-reseed under test. The fixture declares the per-turn statuses
(`failed`, then `completed`) and the floor enforces them positionally, along
with a single trace covering both turns (harness-seeded turns chain into the
originating send's trace).

The fixture tests pin:

- the streamed frame sequence and terminal response agreement;
- function-call and function-result history for E2E-002;
- the Console-specific system-prompt and `agent_trigger` tool matchers;
- serialization round trips and the authoritative `harness::send` schema.

## Runtime and evidence

The direct lifecycle is allocate → boot → arm → send → await → collect →
grade → teardown → report. Playground replaces send with an externally
initiated Console or SDK turn and waits for shutdown after completion.

- Harness readiness, completion, and trace stabilization are awakened by iii
  triggers. A bounded boot-only discovery barrier verifies the authoritative
  function surface because the pinned engine does not replay its current
  registry to late subscribers.
- All RPCs and event waits use bounded monotonic deadlines.
- The observability worker captures every session trace with 100% sampling.
- Child processes run in dedicated process groups and teardown uses SIGTERM
  followed by SIGKILL within one hard cleanup budget.
- Router matching is explicit for all request fields.

Direct runs write `result.json`, `execution.json`, `teardown.json`, and
`stack.json` below `target/integration/<run-id>/`. Scenario evidence includes
the transcript, status, router calls, and complete session trace trees.
`traces.json` is the execution oracle for controlled functions and lifecycle
delivery. `result.json` is stable across runs because concrete run, session,
and turn ids are scrubbed from failure text; `execution.json` records the
SHA-256 of those exact result bytes.

Playground runs use `target/console-e2e/<run-id>/` and additionally write
`playground-ready.json` and `playground-result.json`. The result contains only
a compact trace summary; full spans remain in the scenario artifact. Passing
runs keep compact reports and remove heavyweight stack state.
