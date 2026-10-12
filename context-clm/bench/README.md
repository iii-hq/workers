# context-clm bench

Manual validation for the `context-clm` strategy (spec §7). Not run in CI: it needs an Anthropic API key and spends
money. Every paid run prints a cost estimate first and refuses to start without `--yes`.

| file | what |
|---|---|
| `ctxbench.py` | ContextBench-style generator (KV store, log triage, needle retention) |
| `skills.py` | per-arm context skill appended to the task instruction |
| `run.py` | episode grids, cost estimate, driver, graders, long coding tasks |
| `report.py` | markdown tables and the pass checks |
| `truth-fields.jsonl` | exact answers of the code-reading task (24 ide + 16 harness functions at `d3f9bcb8a`) |
| `arms/` | bench-only worker: `context-sum::assemble` (summary at 75% of the budget) and `context-base::assemble` (no management) |
| `boot.sh`, `down.sh` | isolated stack on `ws://127.0.0.1:49250` (never the live `:49134`) |

Arms: `clm`, `sum`, `base` opt their session in with session metadata
`{"context_strategy": "<arm>", "context_budget_tokens": <budget>}`; `default` sets neither (context-manager, no wall).

## Setup (once per checkout)

Run from the repository root of the checkout that holds this bench.

```bash
python3 -m venv context-clm/bench/.venv
context-clm/bench/.venv/bin/pip install tiktoken==0.14.0
(cd context-clm/bench/arms && pnpm install --frozen-lockfile)
(cd harness && cargo build --release --locked)        # its build.rs runs pnpm in harness/ui
(cd context-clm && cargo build --release --locked)
for w in state queue session-manager context-manager cron llm-router provider-anthropic ide; do
  (cd /home/anderson/project/iii/workers/$w && cargo build --locked)
done
```

The last loop builds the unchanged workers in the main checkout (`MAIN`, override with `MAIN=/path`), which
`boot.sh` copies.

Offline checks (no stack, no API calls):

```bash
cd context-clm/bench && .venv/bin/python test_bench.py && (cd arms && pnpm test)
.venv/bin/python ctxbench.py --check kv:1000:0 kv:4000:0 log:1600:0 log:3200:0 needle:24:0 needle:60:0
```

## Stack

```bash
export RUN=/home/anderson/context-clm-runs/bench-01   # outside any git repo; one directory per bench run
export ANTHROPIC_API_KEY=...
context-clm/bench/smoke.sh                     # optional, no model call: boot, check the wiring, tear down
context-clm/bench/boot.sh                      # logs in $RUN/logs, sessions in $RUN/sessions
context-clm/bench/down.sh                      # when done
```

`boot.sh` refuses to start while something listens on `:49250` (another run's stack: stop it with `down.sh` and its
`RUN`). ide execs find `$RUN/shim/iii` first on `PATH`: it runs only `iii trigger`, pinned to the bench engine, so an
exec never reaches the live `:49134` through the CLI's default URL. `run.py` refuses a `BENCH_ENGINE` on `:49134`.

## Regression bench (spec §7.1)

```bash
cd context-clm/bench
.venv/bin/python run.py grid regression > $RUN/regression.json
.venv/bin/python run.py run $RUN $RUN/regression.json            # prints the estimate, starts nothing
.venv/bin/python run.py run $RUN $RUN/regression.json --yes --workers 6
.venv/bin/python report.py $RUN > $RUN/report.md; .venv/bin/python report.py $RUN --check
```

## Long coding tasks (spec §7.2)

| task | source | snapshot | graded by |
|---|---|---|---|
| `http-brace` (small feature) | `6183f131d` (#1294) | `e5775dbf9:{http,crates}` + the PR's tests | `cargo test --locked --lib` and `--test webhook_listener --test e2e_routing` in `http/` |
| `ide-update-op` (bug fix) | `325a19129` (#1321) | `0466e008a:{ide,crates}` minus `ide/ui` + the PR's tests | `cargo test --locked --test code_update_ops --test code_golden_errors` and `--lib update_file` in `ide/` |
| `fields` level 40 (code reading) | — | `d3f9bcb8a:{ide,harness}/src` | exact answer vs `truth-fields.jsonl` |

Before cargo runs, the grader puts back from the fix commit everything that steers the graded tests (the spliced
unit-test modules, the crate's `tests/`, `Cargo.toml`, `Cargo.lock`, `build.rs` and `rust-toolchain.toml`) and deletes
`.cargo/` dirs, `build.rs` and `rust-toolchain{,.toml}` files the fix commit does not have, so editing them does not
help. Cargo then runs with a minimal environment (`PATH`, `HOME`, `CARGO_HOME`, `RUSTUP_HOME`, `LANG`, no API keys).
An episode is not graded when its snapshot held a `.cargo/config` or `config.toml` that the fix commit does not have,
or when `~/.cargo/config` or `config.toml` differs from the one `run.py` started with, before cargo runs or after it
(a parallel episode may change it meanwhile). Its result then has score 0 and `grade_refused: true` with the reason,
also in its `results/<sid>.grade.log`. `report.py` counts refused grades in their own column. Check that each coding
task fails before its fix and passes after it (about two minutes, no API calls):

```bash
.venv/bin/python run.py verify-long $RUN
```

Pilot first, then the full grid priced from the pilot (`run` skips and does not price episodes that already have a
result in `$RUN/results`, so the pilot episodes are not paid twice):

```bash
.venv/bin/python run.py grid long-pilot > $RUN/long-pilot.json
.venv/bin/python run.py estimate $RUN/long-pilot.json
.venv/bin/python run.py run $RUN $RUN/long-pilot.json --yes
.venv/bin/python run.py grid long > $RUN/long.json
.venv/bin/python run.py run $RUN $RUN/long.json --prior $RUN            # prints the estimate, starts nothing
.venv/bin/python run.py run $RUN $RUN/long.json --prior $RUN --yes
.venv/bin/python report.py $RUN > $RUN/report.md; .venv/bin/python report.py $RUN --check
```

## Pass criteria

- §7.1: `clm` scores at least the spike's per group (`SPIKE_CLM` in `report.py`) and zero
  `anthropic dropped replayed thinking blocks` events on `clm` sessions. If the `ctxbench.py` docstring says its
  wording is our own, the paper's licence forced a rewrite and `SPIKE_CLM` was measured on other formats. A miss
  still fails the check; say "not like-for-like" when you report it.
- §7.2, per model: `clm` solves at least as many tasks as `default` minus one (a task is solved when most of its
  seeds pass), with a lower mean peak prompt and a mean cost no higher than `default`.
- `report.py --check` also fails on a crashed episode with no result since, on a refused grade, on a group or long-task
  model with rows for another arm but none for `clm`, and on a `clm` row whose provider log was not found (its drops
  are unknown). `run` refuses to rerun a crashed sid, so rerun the episode with a new `tag`: a result with the same
  arm, task, level, seed and model clears the crash.

## Live dogfooding (spec §7.3, after both PRs merge)

The live stack is `iii compose` in namespace `my-project`, run from `/home/anderson/project/iii/workers/template`
(logs in `template/.iii/compose/my-project/logs/`). Never restart the whole project; restart one container at a time.

1. Update the main checkout and build: `git -C /home/anderson/project/iii/workers pull --ff-only` then
   `(cd /home/anderson/project/iii/workers/context-clm && cargo build --locked)`.
2. Ask the user for a quiet window (no running turns), then restart only the harness so it runs the strategy
   selection: `iii trigger compose::restart --namespace my-project --json '{"container":"harness"}'`.
3. Add the worker (writes `template/worker-compose.yaml`; never commit it):
   `iii trigger compose::add --namespace my-project --json '{"workers":[{"worker":"../context-clm","start_after":["context-manager"],"environment":{"RUST_LOG":"info"},"scripts":{"run":"cargo run --bin context-clm"}}]}'`
   and check: `iii trigger compose::status --namespace my-project | jq '.containers[] | select(.container == "context-clm")'`
   shows `state: ready`, and `iii trigger engine::functions::info --json '{"function_id":"context-clm::assemble"}'`
   answers.
4. Opt a session in. `session::set-meta` replaces the whole metadata object, so merge:

   ```bash
   SID=<session id the user picks>
   META=$(iii trigger session::get --json "{\"session_id\":\"$SID\"}" | jq -c '(.meta.metadata // {}) + {context_strategy: "clm"}')
   iii trigger session::set-meta --json "{\"session_id\":\"$SID\",\"metadata\":$META}"
   mkdir -p /home/anderson/context-clm-runs/dogfood
   echo "$SID $(date -u +%FT%TZ)" >> /home/anderson/context-clm-runs/dogfood/sessions.txt
   ```

   After its next turn: `iii trigger harness::status --json "{\"session_id\":\"$SID\",\"verbose\":true}" | jq .context_strategy`
   shows `effective: true` (or the `skipped_reason`).
5. Daily, for each line `SID SINCE` of `sessions.txt`:

   ```bash
   L=/home/anderson/project/iii/workers/template/.iii/compose/my-project/logs
   S=/home/anderson/project/iii/workers/template/data/session-manager   # live session store
   # budget_tokens is the session's effective budget; 2048 is the worker's default reserve_tokens
   iii trigger context-clm::status --json "{\"session_id\":\"$SID\"}" |
     jq -c '{budget_tokens, limit: (.budget_tokens - 2048), stopped} + (.stats | {peak_tc, edits_applied, edits_rejected, rollbacks, stops})'
   grep "anthropic dropped replayed thinking blocks session_id=$SID " $L/provider-anthropic.log | awk -v t="$SINCE" '$2 >= t' | wc -l
   grep -c 'harness.context_budget_stop\|harness.context_strategy_unavailable' $S/$SID.jsonl
   grep -c ERROR $L/context-clm.log
   ```

   Drops before `SINCE` are not CLM's (default sessions also log some). Turn failures are `error` entries with a
   `code` in the session file. Step logs are under `<III_COMPOSE_DIR>/data/context-clm/log/` (the project's `data/context-clm`).
6. Pass after about a week:
   - zero `harness.context_budget_stop` / `harness.context_strategy_unavailable`;
   - zero `ERROR` lines in `context-clm.log`;
   - zero drop events after opt-in;
   - `stats.stops == 0`, and on sessions with `stats.rollbacks == 0`, `stats.peak_tc` never above `limit`
     (rollbacks are reported, not failed: a rollback means the context went over and was cut back);
   - on every session whose `stats.peak_tc` crossed half of `budget_tokens`, `stats.edits_applied` above 0 and
     `stats.edits_rejected` below `stats.edits_applied`;
   - and the user's verdict on quality.
7. Rollback: drop the key (`jq -c '(.meta.metadata // {}) | del(.context_strategy)'` in step 4), or remove the
   worker with `iii trigger compose::remove --namespace my-project --json '{"workers":["context-clm"]}'` (opted-in
   sessions then fail with the removal hint). Never `compose::stop`: it stops every project and exits the daemon.
