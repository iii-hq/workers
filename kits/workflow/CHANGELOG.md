# Changelog

## 0.1.0

- **New kit `iii/workflow`.** The ticket strategy of `iii/deepwork` (a run ticket, subtask cards, outcome labels, bounded rounds, a critic and a synthesized report) with every wave as one `workflow::start` run.
- **Phases are workflow nodes.** Each ticket's work and check are nodes with their own profile and model; a check's `fail` loops the work back with its reasons, up to `attempts` passes; a stuck node times out without stopping its siblings.
- **Design before tickets:** a design run explores the goal from three angles, writes three competing designs (minimal, robust, fit), refutes each, and a planner turns the survivors into the plan and the tickets; the review refutes each of its findings before it becomes a ticket. `design=auto` (the default) runs it only when the goal leaves the approach, the place or the size open, and records the decision and its reason on the run ticket; `on` always runs it, `off` never.
- **Dev flow:** a baseline node, one worktree per code ticket, integration on a `workflow/<slug>` branch, one tests-and-check run per integrated wave, repaired first in the next round when it fails, and a kept branch.
- **A board people can read:** the run ticket, cards and comments are markdown with short sections, checklists and short shas, and a goal is planned as a few reviewable tickets instead of many small ones.
- **The workflow is the record, the board the summary:** items stay in To do while they run (no `in_progress` moves), the board is read once per round, a retry writes nothing, the run pointer is one `state::set` per run, and `board=off` runs a small goal with no board at all.
- **Commit identity from git, never typed:** a work node reports the sha `git rev-parse HEAD` prints, and the check resolves the claimed sha with `git rev-parse --verify`, so a short or mistyped sha never fails correct work; commit messages and trailers are never criteria.
- **Installs beside deepwork:** its own profiles (`workflow*`), label `wf:run`, branches, worktrees and state scopes.
- **Requires** a `workflow` worker with the flat `workflow::start` request (`nodes` and `output` at the top level; `task`, `reads`, `schema` and `fanout` on a node), node options (`profile`, `model`, `label`, `phase`, `loop`, `filesystem_root`), an `output` that may name a loop body node, the caller folder, and the run-end notification: `workflow::start` returns the `harness::observe` call (`next`) for `workflow::on-run-completed` with args `{run_id}`, which replays a run that already ended.
