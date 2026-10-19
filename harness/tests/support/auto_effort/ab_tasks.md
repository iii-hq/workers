# Level cost A/B: preregistration

Auto-effort phase 0, Task 6. This file is frozen before the first priced call (`ab.py --pilot --run`). A bar, arm, task,
check or stop rule changed after that call makes a new A/B with a new preregistration. The sections follow the gstack
`/eval-plan` preregistration (Bars, Arms, Decoys, Stop rules, Held-out exposure), after the tasks and the run setup.

## What it measures

- `ratios.json`: per model, the mean `cost_units` of a run at `low`, `high` and `xhigh`, each over the mean at `medium`.
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
| b | easy edit | Rename `units` to `cost_units_of` in harness/tests/support/auto_effort/common.py and update every caller. | `common.py` holds no `def units(`, and TEST_CMD passes in the tree |
| c | hard fix | test_p95 in harness/tests/support/auto_effort fails; find and fix the bug without changing the test. | TEST_CMD with `-k test_p95` passes, and `test_auto_effort.py` is unchanged |
| d | hard investigation | Why does a harness send that omits thinking_level keep the previous level? Cite file:line. | the answer contains `inherit_prior_reasoning` and `reasoning_fields_omitted` |

- The answer is the text of the session's last assistant message.
- TEST_CMD is this package's unit test command. Task b runs it instead of a cargo build, so a run needs no cold build.
- Before task c, `ab.plant` does two things (R6):
  - It moves `analyze.p95` one rank down: `- 1]` becomes `- 2]` in the nearest-rank index.
  - It writes the sha256 of `test_auto_effort.py` to `<tree>.test-sha256`. That file sits beside the tree,
    outside the session's filesystem scope.
- Task c's check runs only `test_p95` (R6). On this package, as `test_plant_then_check_fix` runs it, the full suite would
  run that test, and so the check, again.

## Run setup

- There are 96 runs: 2 models × 4 levels × 4 tasks × 3 runs (R13).
  - They run in this order: model, then task, then run, then level. Drift over the hours of a run then hits the four
    levels alike.
  - The pilot's four runs are claude-opus-5-5 at `medium`, run 0 of each task. They belong to the full run, so a
    `--run` after the pilot adds the other 92.
- Each run has four steps:
  1. Build the run's tree (`ab.build_tree`, R15) in a new directory,
     `out_dir/trees/ab-<model>-<level>-<task>-<run>-<random>`: `git -C <repo> archive --format=tar <sha>`, extracted
     with `tarfile` (`filter="data"`), then `git init -q`, `git add -A` and one commit, `base`, by `ab <ab@localhost>`.
     Then `plant`. The sha is resolved once, so every tree holds the same files.
  2. One `harness::send` on a new `e2e` session, which keeps it out of the user corpus:
     `{"message", "model", "provider": "anthropic", "session": {"kind": "e2e"}, "options": {"thinking_level", "functions": {"allow": […]}, "metadata": {"fs_scope": {"root": <tree>}}}}`.
  3. Wait as the cache probe does.
     - Poll `harness::status` every 2 s until the turn ends, for at most 1,800 s. A status call that fails is retried;
       the third failure in a row ends the wait with that error.
     - Then re-read the session file under `--data` until the last assistant message carries its usage, for at most
       10 s.
     - A turn that did not complete, or whose wait a Ctrl-C interrupted, gets `harness::stop`, so it stops spending.
  4. Run `check`, write the row to `ab.jsonl`, and remove the tree with `shutil.rmtree`. A removal that fails is
     printed to stderr and the run goes on: every run's tree has a name of its own, a retry's too, so a leftover never
     blocks a later run.
- Isolation (R15): each run's git dir is its tree's own `.git`, so no other branch or stash is reachable from the
  session, and no hook of the user's repository runs. A linked worktree would share the user's `.git` (stash, refs,
  hooks) and reach every branch, the checks included.
- Some runs carry an `error`: the turn failed, was cancelled or was stopped, or a send or status call failed.
  - Such a run is a fail (`pass` is false), whatever its tree or answer holds: the turn did not finish the task.
  - When it spent (`cost_units > 0`), it stays in `ab.jsonl` as a cost sample for `ratios` and is never re-run:
    timeouts cluster at high effort, so dropping or replacing them would bias the ratios low (R17).
  - When it spent nothing, it is no cost sample. The next `--run` runs it again and replaces its row.
  - Rows without an error are never re-run. Delete `ab.jsonl` to start over.
- The stack is the template stack (`iii trigger -n my-project`) with `harness`, `llm-router`, `provider-anthropic`,
  `session-manager` and `ide`.
  - Its harness config sets `filesystem_boundary: workspace`, so `coder::*` calls stay inside the tree.
  - Its ide config (`exec_confinement: landlock`) lets `shell::exec` write only to the tree, whose git dir is inside
    it, `/tmp` and tool caches.
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
- Setup sanity: the pilot (opus at `medium`, each task once) runs with no errors and passes tasks a and b. A miss stops
  the A/B (see Stop rules).

The G3 thresholds are `analyze.py`'s preregistered constants (`G3_SAVING`, `G3_COST_CAP` and `G3_HARD_GAIN`). This A/B
produces their inputs.

## Arms

There are eight arms, one per model and `thinking_level`. Every arm uses provider `anthropic`, the send options above,
and 12 runs (4 tasks × 3).

- The cost estimate is about 100,000 units a run (`RUN_UNITS`), at every level.
  - In the corpus, child tasks have a median of 25,000-33,000 units, and opus top-level turns a median of about 126,000
    units over 8 steps.
  - `gstack-eval-plan arms --input-tokens 90000 --output-tokens 2000` priced it at the catalog's input and output prices
    (`provider-anthropic/src/curated.rs`).
- `xhigh` is measured (R13), so G3 can price a pick or a corpus row at `xhigh`. Its runs likely cost more than the
  estimate, which the pilot, at `medium` only, does not measure.

| Arm | Model | thinking_level | Runs | Cost per run |
|---|---|---|--:|--:|
| opus-low | claude-opus-5-5 | low | 12 | about $0.40 |
| opus-medium | claude-opus-5-5 | medium | 12 | about $0.40 |
| opus-high | claude-opus-5-5 | high | 12 | about $0.40 |
| opus-xhigh | claude-opus-5-5 | xhigh | 12 | about $0.40 |
| sonnet-low | claude-sonnet-5-5 | low | 12 | about $0.20 |
| sonnet-medium | claude-sonnet-5-5 | medium | 12 | about $0.20 |
| sonnet-high | claude-sonnet-5-5 | high | 12 | about $0.20 |
| sonnet-xhigh | claude-sonnet-5-5 | xhigh | 12 | about $0.20 |

### Spend estimate

| Step | claude-opus-5-5 | claude-sonnet-5-5 |
|---|--:|--:|
| Pilot (`--pilot --run`) | 4 runs, about 400,000 units, about $1.60 | none |
| Full run (`--run`, after the pilot) | 44 runs, about 4,400,000 units, about $17.60 | 48 runs, about 4,800,000 units, about $9.60 |

- All 96 runs cost about $28.80: the earlier estimate's rate, $21.60 for 72 runs or $0.30 a run, scaled to 96.
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
  - The dry run prints `4 runs` with `--pilot` and `96 runs` without it.
- The pilot is paid and needs the user's OK. It runs
  `python3 -I ab.py <out_dir> --sha <sha> --data <stack data dir> --pilot --run`. Its four runs, opus at `medium` on
  tasks a-d, go through the read, edit and exec paths once each (R13).
  - If any pilot run has an `error`, the A/B stops until the plumbing (the stack, `--data`, the functions or the tree)
    is fixed. Then delete `ab.jsonl` and run the pilot again: a run that errored after spending is never re-run, and
    a broken setup's runs are no samples of an arm.
  - If the pilot fails task a or task b, the easy tasks at `medium`, the A/B stops for investigation. Every arm shares
    the setup, so the full run waits too. A fail on c or d, the hard tasks, does not stop it.
  - If the pilot's mean cost is over 200,000 units a run (twice the estimate), re-estimate the full run and ask again.
  - Otherwise, show the user the spend table and wait for approval before the full run.
- The full run is paid and needs the user's OK. It is the same command without `--pilot`.
  - Stop it when either model's summed `cost_units` passes twice its approved estimate. Ctrl-C stops the running turn
    and keeps the rows of finished runs.
  - Runs that errored before spending (`cost_units == 0`) go again on the next `--run`, once. If an arm still has such
    runs after that, report instead of retrying. A run that errored after spending is kept as a cost sample and a fail,
    and never goes again.
- Any `--run`, pilot or full, stops by itself after 3 runs in a row with an error, so a broken setup does not pay for
  every run. Fix the setup, and delete the errored rows it wrote (the last rows of `ab.jsonl`), before the next
  `--run`: a spent one would otherwise stay as a sample.
- A turn that has not ended after 1,800 s is stopped, and its run is recorded with the error.

## Held-out exposure

| Set | Prior exposure | Reservation |
|---|---|---|
| The effortless holdout (20 cases in `cases/effortless-judge-cases.json`) | judged by the Task 3 arms for G1 | `ab.py` never reads it, and no A/B prompt comes from it |
| The corpus prompts (`prompts.jsonl`) | labelled and judged in Tasks 2 and 3 | `ab.py` never reads them |
| The four A/B prompts and their checks | none: they were written for this A/B | judged by the Task 3 arms through `ab_prompts.jsonl` (R4), and never labelled; the runs' trees hold a commit without them |
