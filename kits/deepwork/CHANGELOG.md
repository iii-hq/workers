# Changelog

## 0.4.2

- A lead no longer copies its ticket's result into the document it writes for the orchestrator. Copies of large results came back `null` or cut down to a pointer, so the synthesis and the final report lost them. The lead's document carries `result_from` (the scope and key of the phase document with status `done`), and the orchestrator, the synthesis card's Inputs and the report read the result there with `state::get`, which the orchestrator now preloads. The synthesis runs when at least two done tickets have a `result_from`.
- The run's final comment opens with a summary of the synthesis report (its citations kept) and the `state::get` that reads the full report, instead of a retyped copy that came out shortened anyway.
- A lead writes its document as a JSON object, never a string that holds one; the orchestrator reads such a string as the object it holds.
- With `plan_only=on` the run's last line is exactly `plan on <R>: <n> tickets; start with args: resume=<R>`.
- The README's approval rules apply only to a project that runs the `approval-gate` worker.

## 0.4.1

- The tests, code and work phases follow the long-run recipe too, so a ticket no longer fails at its first build in a fresh worktree. 0.4.0 kept them on `shell::exec`, capped at 120 s, and the leads' child deny list blocked `engine::register_trigger`, so a cold Cargo build was killed before any test ran. The deny list no longer includes it, the recipe now names a cold first build and any command that hit the cap, and `deepwork-worker` (the base of `deepwork-impl`) preloads `shell::exec_bg`, `shell::status`, `shell::kill` and `engine::register_trigger`. A Verify line that is a long run keeps its command, args and `cwd`.

## 0.4.0

- A parked parent now reacts when a child ends: a lead whose phase child ended without its result fails that attempt at once (`no result: <status>`), and the orchestrator closes the key of a lead that ended without one so the wave barrier still closes. Both use the `child ended` notice that a harness sends to a parked parent; older harnesses never send it, and the deadline wakes apply as before.
- The busy check no longer reads a `completed` session with `expects_wake:true` as working. The orchestrator resumes a lead whose phase child has ended and whose key is already in the scope (a wake that never arrived) in the same session with a `Resume:` task; the lead handles that task and carries `engine::unregister_trigger` and `harness::triggers::list`.
- The run stops, naming the folder to start from, when the files the goal or its ticket names (or the repository it names) are not under the repository root. It never searches the disk for the repository.
- Long commands run as background jobs. The baseline, the branch review's full suite and linter, and any verify step that needs more than ~90 s use `shell::exec_bg` and a `shell::job-finished` wake instead of `shell::exec`, which is capped at 120 s: the agent registers the wake, ends its turn, and reads the exit code when it fires. Every lead task ends with the recipe. Verify spawns now allow `shell::exec_bg`, `shell::status`, `shell::kill` and `engine::register_trigger`, and the critic's deny list no longer includes `engine::register_trigger`. Requires `ide` `>=0.12.19`, the first synced release known to carry `shell::job-finished` and its replay.
- Review checks behaviour, not only scope: for each new branch, state, effect or caller it names an input that would break it and reads the code that handles it. A confirmed defect is a rejection with the reason `<file>:<line>: <input> → <observed wrong behaviour>`, and a reason that starts with `test:` makes the lead run the tests step again first.
- A defect in the run's own diff is a new item, never a loose end. Workers and the critic report it as a `new_item` (a fix card whose `Test first:` is the input that shows it), and a worker fixes a one-line defect inside its own `Files:` in its phase.
- Loose ends are one line each (`<file>:<line>: …`) and no longer become tickets. They are kept for the report, one that names an existing ticket becomes a comment on it, and the finish creates one `Follow-ups: <goal>` subtask in Backlog, labelled `pending`, with a checklist (`- [ ] <line> (<source>, <commit>)`). The round comment's `loose <n>` counts lines.
- Existing tickets are adopted, not re-created. A live ticket the goal, the run ticket or its comments name becomes a subtask of the run ticket with its card appended; a card that would open with `Fixes <KEY>` or `Part of <KEY>` is that ticket's card; a goal that is itself a ticket key makes that ticket the run ticket. The design step asks only when the user's text and the board disagree, and added work in a replan follows the same rule.
- The finish card gains "Open a PR and merge when CI is green". Both PR options then run a Land step: `gh pr checks --watch` as a background job (up to an hour), a squash merge on green for the merge option, one `Fix CI: <check>` ticket and a round when a check fails and rounds remain, and `CI: <state>, PR open` otherwise. A repository with no checks counts as green. The card holds at most four options (`harness::ask`), so discarding the branch is now the typed answer `Discard`. A merge happens only on the user's finish answer, and a run never deploys or writes to production unless the latest message asks for it.
- Dev flow runs call `browser::doctor` once when the goal needs a browser and the function exists; on `ok:false` the Plan card carries `preflight: browser: …` and the run goes on.
- The cost line is `≥ $C` from `totals.cost_usd`, with `incomplete (<provider/model> unpriced)` for each `by_model` row that has `cost_complete:false`, or `cost unknown` when the total is null. The flag takes effect with a harness that reports `cost_complete`; an older harness reads as before, minus the fallback that summed `by_session`.

## 0.3.0

- One worktree per code ticket (`isolation=ticket`, the default in the dev flow): each impl-tier ticket is dispatched into its own worktree `.worktrees/<slug>-<key>` on a branch off the run branch, rebased on it for a retry (a conflicting rebase starts the ticket over in a fresh worktree, with the old commits kept on a reference branch until Finish), and landed on it with a cherry-pick once verified, or accepted with verify off; a conflict is a rejection retried next round on the new base. Tickets that share a file or a test target no longer wait for a later wave, and commit hooks no longer force one ticket per wave. `isolation=shared` keeps the 0.2 behaviour.
- `approve=on` now also gates open design questions and the plan in the plain flow, not only in the dev flow. `plan_only=on` plans without dispatching: the run ticket and its cards go on the board, and `resume=<R>` starts the run later.
- `synthesize=on` (the default in the plain flow) ends a run, when at least two tickets are done with a result and the run was not stopped, blocked or over budget, with one `Synthesize:` ticket that merges their results into a single report, every claim citing its ticket, verified like any other ticket; the report opens the run's final comment. The planner no longer plans its own merge item.
- `resume=<R>` adopts an earlier run from its board state: goal, args, flow, branch and worktree come from the run ticket, In progress tickets whose lead is gone are re-queued (a lead that is still running is stopped first), and the rounds continue. A finished run resumes the same way once cards are back in To do.
- Dev cards' `Verify:` lines carry no `cwd`: every agent runs them in its own worktree. A fresh ticket worktree gets the install step the repository needs before it builds (a Node project's offline install); build output is never shared between worktrees.
- The verifier treats a test command that ran no test (`0 passed`) as satisfying nothing.
- Lead spawns spell out the 48-character limit of the session name (the key and ` · ` count), which runs with long ticket titles hit.

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
