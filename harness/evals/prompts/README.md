# Prompt evals

Benches system-prompt variants against a live rig and grades agent conduct, not
exact text: what the agent called, in what order, what it avoided, and what the
final result contained. Use it before changing any identity prompt, playbook
skill, or injected guidance.

## What it is (and is not)

- `harness/evals/integration` is the deterministic conformance suite: isolated
  stack, scripted `router::*`, exact invariants, CI gate. A scripted router
  cannot measure prompt-driven behavior, which is exactly what changes when a
  prompt changes.
- This runner is the complement: live engine, live provider, live prompts. It
  answers "does the smaller prompt still produce correct conduct" with a task
  matrix instead of a diff review.
- The HarnessBench tech spec describes the full product (matrix runs, metrics
  store, console view). This is its working seed: one file, zero dependencies,
  markdown + JSON reports.

## Requirements

A running engine with harness, session-manager, context-manager, a real
provider, and the `iii` CLI on PATH. The fp worker should be connected if you
run the bulk scenario (it grades pipe conduct).

## Usage

```
node run.mjs                                  # all arms x all scenarios
node run.mjs --scenario bulk-two-fields       # one scenario, both arms
node run.mjs --arm candidate                  # one arm
node run.mjs --address <engine-host>          # non-default engine
```

Exit code 0 when every non-optional assertion passes; reports land in `out/`
(gitignored) as `report-<runid>.md` and `report-<runid>.json`, including each
arm's measured prompt tokens (`context::count-tokens`) and the full call
sequence per session.

## Arms

`scenarios.json` defines the arms:

- `current` sends no override, so the session runs whatever the router serves
  the rig today (provider identity or operator override). Its token count is
  fetched live from `router::system_prompt::get`.
- `candidate` reads `harness/prompts/default.txt` from this checkout and sends
  it with `system_prompt_strategy: override`.

Point `system_prompt_file` at any file to bench a different variant; add more
arms for a multi-way comparison.

## Scenarios and assertions

Each scenario is a user message plus assertions:

- `result_matches` - regex over the final turn result.
- `calls_include` / `calls_exclude` - regex over the invoked function ids
  (transcript order, including function results).
- `call_order` - the first match of `before` must precede the first match of
  `after`; passes when `after` never fires.
- `arms` scopes an assertion to specific arms; `optional: true` records the
  outcome without failing the run.

The shipped matrix covers the conduct that prompt changes have actually
regressed or nearly regressed: no orchestration machinery on simple tasks,
playbook pull before spawning, bulk payloads moved with `fp::pipe` instead of
flooding the context (a 1 MB fetch read inline costs roughly 270k tokens), and
no installs on a read-only registry question.

## Notes

- Sessions are named `pbench-<runid>-<arm>-<scenario>`; they stay on the rig
  afterwards and are inspectable in the console.
- Every run sends `functions: { allow: ["*"] }`: a parentless `harness::send`
  denies all dispatch by default, which would fail every scenario.
- Fan-out scenarios settle on a quiet window (no active turn for `quiet_s`
  seconds) because reactions can wake a session after its first turn completes.
