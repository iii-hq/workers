# Level cost A/B: preregistration

Auto-effort phase 0, Task 6. This file is frozen before the first priced call (`ab.py --pilot --run`). A bar, arm, task,
check or stop rule changed after that call makes a new A/B with a new preregistration. The sections follow the gstack
`/eval-plan` preregistration (Bars, Arms, Decoys, Stop rules, Held-out exposure), after the tasks and the run setup.

## What it measures

- `ratios.json`: per model, the mean `cost_units` of a run at `low` and at `high`, each over the mean at `medium`.
  `analyze.py` scales corpus turns by these ratios for G3's modeled cost.
- `ab.jsonl`: one row per run, `{model, level, task, run, pass, cost_units, steps, error}`.
  - G3's quality clause compares the pass rate at the level the G1-winning judge picks for each (model, task) with the
    pass rate at `medium` (R4).
  - G3's gain clause counts the passes on the hard tasks, c and d.
- `ab_prompts.jsonl`: the eight (model, task) prompts as judge rows, for `judge.py --rows` (R4). Each row has
  `id = ab-<model>-<task>`, `slice = ab`, `level = medium` and `context = ""`.

A run's cost is the summed `common.units(usage)` of every assistant message in its session. The weights are input 1,
cache_write 1.25, cache_read 0.1 and output 5, so a unit is one of that model's uncached input tokens. Units of different
models are never added together.

## Tasks and checks

| Task | Kind | Prompt, sent verbatim | Pass when |
|---|---|---|---|
| a | easy lookup | What does `confident_choice` in harness/src/reconcile.rs return when the choice is `none`? | the answer contains `None` |
| b | easy edit | Rename `units` to `cost_units_of` in harness/tests/support/auto_effort/common.py and update every caller. | `common.py` holds no `def units(`, and TEST_CMD passes in the worktree |
| c | hard fix | test_p95 in harness/tests/support/auto_effort fails; find and fix the bug without changing the test. | TEST_CMD with `-k test_p95` passes, and `test_auto_effort.py` is unchanged |
| d | hard investigation | Why does a harness send that omits thinking_level keep the previous level? Cite file:line. | the answer contains `inherit_prior_reasoning` and `reasoning_fields_omitted` |

- The answer is the text of the session's last assistant message.
- TEST_CMD is this package's unit test command. Task b runs it instead of a cargo build, so a run needs no cold build.
- Before task c, `ab.plant` does two things (R6):
  - It moves `analyze.p95` one rank down: `- 1]` becomes `- 2]` in the nearest-rank index.
  - It writes the sha256 of `test_auto_effort.py` to `<worktree>.test-sha256`. That file sits beside the worktree,
    outside the session's filesystem scope.
- Task c's check runs only `test_p95` (R6). On this package, as `test_plant_then_check_fix` runs it, the full suite would
  run that test, and so the check, again.

## Run setup

- There are 72 runs: 2 models × 3 levels × 4 tasks × 3 runs.
  - They run in this order: model, then task, then run, then level. Drift over the hours of a run then hits the three
    levels alike.
  - The pilot's three runs are the first three of that order, so a `--run` after the pilot adds the other 69.
- Each run has four steps:
  1. `git worktree add --detach out_dir/worktrees/ab-<model>-<level>-<task>-<run> <sha>`, then `plant`. The sha is
     resolved once, so every run checks out the same commit.
  2. One `harness::send` on a new `e2e` session, which keeps it out of the user corpus:
     `{"message", "model", "provider": "anthropic", "session": {"kind": "e2e"}, "options": {"thinking_level", "functions": {"allow": […]}, "metadata": {"fs_scope": {"root": <worktree>}}}}`.
  3. Wait as the cache probe does.
     - Poll `harness::status` every 2 s until the turn ends, for at most 1,800 s.
     - Then re-read the session file under `--data` until the last assistant message carries its usage, for at most
       10 s.
     - A turn that did not complete gets `harness::stop`, so it stops spending.
  4. Run `check`, write the row to `ab.jsonl`, and remove the worktree with `git worktree remove --force`.
- Some runs carry an `error`: the turn failed, was cancelled or was stopped, or a send or status call failed.
  - Such a run is no cost sample for `ratios`.
  - The next `--run` runs it again and replaces its row.
  - Rows without an error are never re-run. Delete `ab.jsonl` to start over.
- The stack is the template stack (`iii trigger -n my-project`) with `harness`, `llm-router`, `provider-anthropic`,
  `session-manager` and `ide`.
  - Its harness config sets `filesystem_boundary: workspace`, so `coder::*` calls stay inside the worktree.
  - Its ide config (`exec_confinement: landlock`) lets `shell::exec` write only to the worktree, its git dir, `/tmp`
    and tool caches.
- `--sha` is the parent of the commit that adds this file: `38193a004` on `feat/auto-effort-phase0`.
  - Its tree holds this package as Tasks 1-5 left it, with the four task targets.
  - It does not hold `ab.py`, this file or the A/B tests. They spell out every check, so they would hand a session its
    answers: a search for `thinking_level` alone finds task d's row above.
  - `ab.py` refuses a sha whose tree holds `ab.py`, and `plant` refuses a tree whose `analyze.p95` line differs.

### Send options: `options.functions.allow`, not `options.agent`

The send uses an explicit allow list of exactly what the tasks need:

| Function | Used for |
|---|---|
| `coder::read-file`, `coder::search`, `coder::tree`, `coder::list-folder` | reading the repository |
| `coder::update-file` | editing it (tasks b and c) |
| `shell::exec` | running the tests (tasks b and c) |
| `engine::functions::info` | the contract before a function's first call |

Why not `options.agent`:

- The console's default profile is the bundled `default` (`iii-directory/prompts/default.md`), and it declares no
  `functions`.
  - A send with `options.agent` and no `options.functions` therefore gets the stack's `default_functions` baseline.
  - On the template stack (`config/my-project-harness.yaml`), that baseline is `allow: ["*"]`: every function, not
    exactly file read/edit and shell.
- `*` includes `harness::spawn`. A sub-agent's cost lands in its own session file, which the runner does not read, so a
  level that delegates more would look cheaper.
- `*` also includes the compose and engine control plane of the shared stack.
- The directory resolves a profile at send time, so an edit under `agents/` during the A/B would change the identity
  between runs. The allow list sits in `ab.py`, at a fixed commit.

Why `engine::functions::info` is on the list:

- For an allow list without `*`, the harness adds a policy aid to the prompt. It lists the allowed ids and tells the
  model to call them directly.
- The aid also says that discovery is denied unless listed, and that a task needing an unknown contract must report
  the blocker and finish.
- The identity prompt requires a function's contract before its first call. Without `engine::functions::info`, the
  contract of `coder::update-file` would be out of reach.
- It only reads contracts.

The identity stays the harness's built-in prompt, with the default `agent_trigger` exposure.

## Bars

- `ratio[model][level]`, the cost at a level over the cost at medium: measured, not gated.
  - A corpus row at a level that has no ratio makes G3 INCOMPLETE.
  - A model with no ratios at all is left out of G3's cost.
- G3 quality clause: the pass rate at the picked levels must be at least the pass rate at medium. Higher is better. A
  miss means the saving clause cannot pass, so Phase 1 is not built for savings.
- G3 gain clause: at least 2 more passes than at medium on the 12 hard runs (tasks c and d × 2 models × 3 runs), at the
  picked levels, for a modeled cost of at most +10%. A miss means there is no quality case for Phase 1.
- Setup sanity: the pilot (opus, task a, one run per level) passes at least 1 of 3, with no errors. A miss stops the A/B
  (see Stop rules).

The G3 thresholds are `analyze.py`'s preregistered constants (`G3_SAVING`, `G3_COST_CAP` and `G3_HARD_GAIN`). This A/B
produces their inputs.

## Arms

There are six arms, one per model and `thinking_level`. Every arm uses provider `anthropic`, the send options above,
and 12 runs (4 tasks × 3).

- The cost estimate is about 100,000 units a run (`RUN_UNITS`).
  - In the corpus, child tasks have a median of 25,000-33,000 units, and opus top-level turns a median of about 126,000
    units over 8 steps.
  - `gstack-eval-plan arms --input-tokens 90000 --output-tokens 2000` priced it at the catalog's input and output prices
    (`provider-anthropic/src/curated.rs`).
- `xhigh` is not measured, so G3 cannot price a corpus turn that the judge sends to `xhigh`.

| Arm | Model | thinking_level | Runs | Cost per run |
|---|---|---|--:|--:|
| opus-low | claude-opus-5-5 | low | 12 | about $0.40 |
| opus-medium | claude-opus-5-5 | medium | 12 | about $0.40 |
| opus-high | claude-opus-5-5 | high | 12 | about $0.40 |
| sonnet-low | claude-sonnet-5-5 | low | 12 | about $0.20 |
| sonnet-medium | claude-sonnet-5-5 | medium | 12 | about $0.20 |
| sonnet-high | claude-sonnet-5-5 | high | 12 | about $0.20 |

### Spend estimate

| Step | claude-opus-5-5 | claude-sonnet-5-5 |
|---|--:|--:|
| Pilot (`--pilot --run`) | 3 runs, about 300,000 units, about $1.20 | none |
| Full run (`--run`, after the pilot) | 33 runs, about 3,300,000 units, about $13.20 | 36 runs, about 3,600,000 units, about $7.20 |

- All 72 runs cost about $21.60.
- The estimate prices every unit at the input price. That overstates opus's cache reads, which cost 0.05 of input
  while the unit weighs them at 0.1.
- The pilot's measured cost replaces this guess.

## Decoys

The checks must reject these outcomes, and the unit tests pin each one:

- Task a, "it returns the key": fails (`test_check_lookup`).
- Task d, an answer that names only one of the two functions: fails (`test_check_investigation`).
- Task c, the planted tree before any fix: fails (`test_plant_then_check_fix`).
- Task c, a tree whose tests pass after an edit to `test_auto_effort.py`: fails (`test_plant_then_check_fix`).
- Task b, an untouched tree: fails, because `common.py` still defines `units`. No unit test calls this check: in a tree
  where the rename is done, the check runs the whole suite, which would hold that test again.

A decoy that passes proves that the check accepts an outcome that did not do the task. Two blind spots are accepted:

- Task a passes any answer that contains `None`.
- Task d passes an answer that only names both functions.

## Stop rules

- Before the pilot:
  - TEST_CMD passes at `--sha`, and at the commit that runs `ab.py`.
  - The dry run prints `3 runs` with `--pilot` and `72 runs` without it.
- The pilot is paid and needs the user's OK. It runs
  `python3 -I ab.py <out_dir> --sha <sha> --data <stack data dir> --pilot --run`.
  - If any pilot run has an error, stop, fix the setup (the stack, `--data` or the functions), and run the pilot again.
  - The plan's rule: an arm at 0/3 on an easy task stops. The pilot holds one run per level, so its three runs are read
    together. If none of them passes task a, the opus arm stops. The full run stops too, because every arm shares the
    setup.
  - If the pilot's mean cost is over 200,000 units a run (twice the estimate), re-estimate the full run and ask again.
  - Otherwise, show the user the spend table and wait for approval before the full run.
- The full run is paid and needs the user's OK. It is the same command without `--pilot`.
  - Stop it when either model's summed `cost_units` passes twice its approved estimate. Ctrl-C keeps the rows of
    finished runs.
  - Runs with an error go again on the next `--run`, once. If an arm still has runs with an error after that, report
    instead of retrying.
- A turn that has not ended after 1,800 s is stopped, and its run is recorded with the error.

## Held-out exposure

| Set | Prior exposure | Reservation |
|---|---|---|
| The effortless holdout (20 cases in `cases/effortless-judge-cases.json`) | judged by the Task 3 arms for G1 | `ab.py` never reads it, and no A/B prompt comes from it |
| The corpus prompts (`prompts.jsonl`) | labelled and judged in Tasks 2 and 3 | `ab.py` never reads them |
| The four A/B prompts and their checks | none: they were written for this A/B | judged by the Task 3 arms through `ab_prompts.jsonl` (R4), and never labelled; the runs check out a commit without them |
