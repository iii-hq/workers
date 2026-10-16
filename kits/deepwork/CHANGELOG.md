# Changelog

## 0.2.1

- Rewrote the README to be shorter and easier to read, and simplified the kit description. No behaviour change.

## 0.2.0

- Renamed the kit and everything it names (0.1.x used earlier names). The kit is `iii/deepwork`, and its profiles are `deepwork`, `deepwork-lead`, `deepwork-worker`, `deepwork-impl`, `deepwork-verifier` and `deepwork-adversary`. Runs carry the `dw:run` label, `dw-<R>-…` state scopes, `dw …` wake labels, `deepwork/<slug>` branches and `deepwork: <goal>` projects. Comment authors and actors are `deepwork*`.
- The repository rule set allows a kanban project create whose `lead` is `deepwork` (rule `deepwork-run-project`).
- Coming from 0.1.x:
  - Finish or stop runs started on 0.1.x first. Their wakes and labels use the earlier prefix, so `continue` cannot resume them.
  - If the install is recorded under the kit's earlier name, it is a separate kit now. Install this one, copy your `model:` pins into `agents/deepwork-*.md`, then remove the old kit with `directory::kits::remove`; its name is in `directory::kits::list`.
  - An `iii/deepwork` 0.1.x install updates in place. Profiles from 0.1.x that you did not edit are removed; edited ones are kept until you delete them, so copy their `model:` lines first.

## 0.1.1

- Reworded the kit description and the README introduction. No behaviour change.

## 0.1.0

- First release: the **deepwork** orchestrator, the hidden **deepwork-lead** that runs one ticket through its phases, the **deepwork-worker** and **deepwork-impl** work tiers, and the **deepwork-verifier** and **deepwork-adversary** check tiers.
- Kanban tickets are the work queue and the live monitor: run items are subtasks of the run ticket (its progress bar tracks the run), each ticket's assignee picks its work tier, each run creates its own kanban project (allowed for a create whose `lead` is `deepwork`; `project=` names an existing one and `release=` an existing release), and tickets dropped from the plan are `canceled` (kanban's native canceled column when the board has one).
- One lead per ticket, shown in the session tree as `<KEY> · <title>` with its phases under it. Every ticket that can run now starts in parallel (`concurrency` defaults to 8; items carry `Depends on` and a dependent waits for a later wave). Leads fan out in bounded waves and fan in through one `state::barrier` wake per wave with a deadline; each lead does the same per phase, retries rejected attempts (`attempts=`), and runs verify and review side by side. Bounded rounds, a completeness critic and an optional soft budget.
- The dev flow, switched on for code goals in a git repository (`flow=auto`). Design and plan approved through `harness::ask` (`approve=on`), test-first code tickets (`deepwork-impl`: `Files`, `Test first`, `Implement`, `Verify`, `Commit`), one git worktree per run with one commit per code ticket, a code review of every commit (research tickets run work then verify, without a commit), a branch review (full suite, linter, whole diff), loose ends filed as `pending` Backlog subtasks, and a finish you choose: merge, PR, keep or discard.
- Steer a run while it works: change the plan in the chat (the design on the run ticket is rewritten first, queued cards follow, a running card gets a follow-up), add or reorder subtasks on the board, comment on a ticket to steer its retry, route a ticket by its assignee. `judges=` verifies each ticket with one judge per model (majority wins); `notify=` sends the final report to a function.
- Each profile preloads the contracts of the functions its role calls on every run (orchestrator: kanban, state, harness spawn/status/stop/metrics/ask, shell and tree; lead: spawn, triggers and state; work tiers: `shell::exec`, `coder::read-file`, plus `coder::create-file` / `coder::update-file` for `deepwork-impl`; check tiers: `state::get`, `shell::exec`).
- Lean context: the orchestrator and leads start without the skills index (`skills_index: false`), ask for compact kanban writes and context-free `harness::metrics`, and phase sub-agents drop the index through their deny list. These need harness 1.8.54 and iii-directory 1.2.40 (the pins below).
- Requires `harness` `^1.8.54`, `iii-directory` `>=1.2.40`, `state` `>=0.22.18`, `ide` `>=0.12.0` and `kanban` `^0.1.21` (subtasks, compact writes, `add_labels` / `remove_labels`). Does not use the `workflow` worker.
