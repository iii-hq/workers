# deepwork

Give **Deepwork** a goal and a team of agents works it, with the kanban board as the task list:

1. Deepwork breaks the goal into tickets.
2. Each ticket gets a lead agent; code tickets have one implementation phase.
3. Leads run in parallel, in waves.
4. Rounds repeat until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

A goal that changes code in a git repository runs the **dev flow**. By default (`approve=auto`), Deepwork decides routine local reversible details and starts the plan without a card; consequential ambiguities still need your decision. Pass `approve=on` for design and plan approval. Each code ticket is implemented in its own git worktree, with one commit, then integrated on the run branch. One batch worker writes the tests for the integrated wave; verification and review run side by side on that combined result. Cards become Done only after their batch passes (`verify=off` skips the check and marks them accepted). The whole branch is reviewed and kept locally by default. An explicit finish choice can merge it, open a PR, or open a PR that merges once CI is green; autonomy never authorizes those actions by itself. Other goals (research, reading, analysis) run the **plain flow**: the plan starts automatically unless `approve=on`, and each ticket is worked and verified. When at least two tickets finish with a result, and the run was not stopped, blocked or over budget, one last ticket merges the results into a single verified report that cites the ticket behind every claim (`synthesize=off` turns it off).

Deepwork needs only the `harness`, `state`, `kanban` and `ide` workers.

## Profiles

| Profile | What it does | Model to pin |
|---|---|---|
| `deepwork` | the orchestrator you talk to | none: uses the session's |
| `deepwork-lead` | runs a ticket or an integrated batch through its phases | a fast, cheap one |
| `deepwork-worker` | research, reading, analysis | optional |
| `deepwork-impl` | implements a card, or writes tests and repairs a batch | a strong one |
| `deepwork-adversary` | verifies and reviews a dev batch, or verifies a plain ticket | a mid-tier one |
| `deepwork-verifier` | base of the checkers, used for judges | none: each judge sets one |

Every profile except `deepwork` is hidden.

## Install

```
iii trigger directory::download-kit kit=iii/deepwork apply=true --timeout-ms 1200000
```

Sometimes the reply is a plan instead of an install. That happens when there are warnings or decisions, for example a profile with the same id already exists. Review the plan and apply it in the ADE under Directory → Kits, or run:

```
iii trigger directory::kits::apply plan_id=<plan_id> --timeout-ms 1200000
```

Applying can take minutes. If the command times out, the apply keeps running, so check Directory → Kits instead of running it again.

### Pin the models

The kit ships without models because it can't know your providers. Add a `model: <provider>::<model>` line to these files:

- `agents/deepwork-lead.md` (optional: without a pin, `lead_model=auto` picks a lighter sibling of the orchestrator's model from the same provider, or the orchestrator's own model when its provider lists no prices; `lead_model=<provider>::<model>` names one from any provider)
- `agents/deepwork-impl.md`
- `agents/deepwork-adversary.md`
- `agents/deepwork-worker.md` (optional)

Use the `provider::model` form so `reasoning_effort` reaches the provider. Kit updates keep your edits.

A tier without its own model uses the one set on Default (`agents/default.md`). If Default has none, it uses the orchestrator's model, also when `lead_model` runs the leads on a lighter one (the leads pass the orchestrator's model to their phases). An unpinned lead spends one turn per phase on that model. With `judges=`, `deepwork-verifier` must resolve no model at all, Default included, or the run won't start.

## Run it

In the console, pick **Deepwork** and type the goal. To change the defaults, add an `args:` line:

```
Add per-user rate limits to the login route.
args: max_rounds=2 concurrency=4 budget_usd=5
```

| Arg | Default | What it does |
|---|---|---|
| `flow` | `auto` | `dev` for code goals in a git repository, `plain` for the rest. Set it to force one |
| `approve` | `auto` | decides routine local reversible choices and starts without a Plan card; asks only for consequential ambiguity. `on` requests design, plan and finish approval. `auto`/`off` keep the branch without a Finish card; neither bypasses permissions or safety boundaries |
| `synthesize` | on | in the plain flow, ends the run with one ticket that merges the done tickets' results into a single report, verified like any other ticket. It runs when at least two tickets are done, and not after a stop, a block or the budget. `off` reports per ticket |
| `plan_only` | off | `on` plans and stops: the run ticket and its cards go on the board, nothing runs. Edit the cards, then start with `resume=<R>` |
| `isolation` | `ticket` | in the dev flow, each code ticket works in its own worktree, branched from the run branch and integrated before batch validation. `shared` runs every ticket in the run worktree. In both, tickets that change the same implementation file wait for a later wave of the round, so they never conflict at integration; reserved test paths may overlap |
| `max_rounds` | 3 | rounds over the queue |
| `attempts` | 2 | tries per ticket lead and per batch lead. A rejected batch retries aggregate tests/repairs and checks, without restarting every card |
| `concurrency` | 8 | leads per wave, at most 8. A ticket waits for its `Depends on:` tickets. With `isolation=shared`, tickets that change the same implementation file never share a wave |
| `verify` | on | checks each dev wave as a batch; plain tickets are checked individually |
| `verifier` | `deepwork-adversary` | the checker profile: `deepwork-adversary*` or `deepwork-verifier*` |
| `judges` | – | comma-separated model ids. One `deepwork-verifier` per model checks each dev batch or plain ticket and the majority wins. At most 7 in the dev flow, 8 in the plain flow |
| `deadline_min` | 20 | minutes each phase gets |
| `critic` | on | one last check for anything missing once the queue is empty. In the dev flow, this is the branch review |
| `max_items` | 5 × `concurrency` | the most tickets a run creates |
| `budget_usd` | – | soft cost cap. It is checked before each wave, and no new wave starts once it is reached. A model without catalog pricing, or a call without usage, isn't counted, so the cap can be overshot |
| `notify` | – | a function id that gets the final report, such as a Slack or email send |
| `project` | – | an existing kanban project (id or name) for the run. Without it, each run creates its own project, `deepwork: <goal> · <id>` |
| `release` | – | an existing kanban release (id or name) for filing cards. Dev validation groups the current wave, not all cards in that release |
| `resume` | – | the key of an earlier run ticket to pick up from the board instead of planning anew (see Resume a run) |

### Hard cost cap

The console only has the soft `budget_usd`. For a hard cap, start the run from the CLI:

```
iii trigger harness::send --json '{"message":"<goal + args>","model":"<provider::model>","options":{"agent":"deepwork","max_cost_usd":5,"max_total_tokens":2000000}}'
```

You must pass `model`, because the profile sets none. Every model the run uses needs catalog pricing, judges included. With `approve=on`, design, plan and finish questions show up in that session in the console. The default `approve=auto` asks only when a consequential decision cannot be resolved from the goal, evidence and repository conventions. Material assumptions go on the run ticket; push, merge, discard and production writes are never implied by autonomy.

A profile's `functions:` list is the contracts it preloads into the prompt, not a permission list. If you restrict the session with `options.functions`, don't copy that list: the run also needs `state::barrier` (the wave and phase wakes are its conditions), `fp::pipe`/`fp::get`, `router::models::list`, `directory::search_functions`, `engine::functions::info`, the `shell::job-finished` wake, and every function its phases use (`coder::*`, `shell::*`, the cards' Verify lines), because a child's policy can't exceed its parent's. The simplest is `"functions":{"allow":["*"]}`, plus your own `deny`.

## The dev flow

1. **Design.** Deepwork reads the repository, chooses the simplest in-scope reversible approach and records material assumptions. It asks one consolidated question with a recommendation only for consequential ambiguity (scope, acceptance, compatibility, security, data loss or external cost); `approve=on` opts into the full design approval flow.
2. **Plan.** The design goes on the run ticket. Each task becomes a subtask with `Files:`, `Tests:`, `Implement:`, `Verify:` and `Commit:` lines. A ticket you named, or that the run ticket mentions, becomes a subtask of the run ticket with its card appended instead of a copy; a goal that is itself a ticket key makes that ticket the run ticket. When the goal needs a browser and the browser worker reports a problem, the card carries a `preflight: browser: …` line; the run goes on. With `approve=auto` or `off`, work starts directly. With `approve=on`, you answer one card: Start, Stop, or type changes; changes rewrite the plan before a new approval.
3. **Worktree.** The run gets a new branch, `deepwork/<slug>`, in `.worktrees/<slug>`. That folder is excluded locally, so your checkout is untouched. Run Deepwork from the repository's top folder; if the files the goal names are outside it, Deepwork stops and names the folder to start from. The test suite, the linter and the typecheck run once, as a background job, to get a baseline; anything already red there is passed to every check as known, so it never fails a batch. Each code ticket then works in its own worktree, `.worktrees/<slug>-<key>`, on a branch off the run branch. A fresh copy may need an install step before it builds (a Node project): Deepwork reads it from the repository and runs it in each ticket worktree. Build output is never shared between worktrees. Code cards defer test and verification commands to the batch in the run worktree, so those builds reuse the run worktree's cache.
4. **Code tickets and batches.** Each ready wave follows this path:
   1. Implement each card in parallel and commit its code. Test paths in `Tests:` are reserved for the batch worker; there is no per-card tests or check session.
   2. Integrate the successful cards on the run branch. Cards that change the same implementation file never share a wave, so their commits apply cleanly; a card that still conflicts (a file its card did not list) retries next round and does not enter this batch. Cards remain In progress.
   3. One batch worker reads the cards and writes their regression tests together in the run worktree, then commits the tests. This is no longer a test-first workflow: it does not prove a failing test before implementation.
   4. Verify and Review run in parallel on the same commit. Verify runs the distinct `Verify:` commands, the full suite and the linter once, keeping every card's acceptance expectations. Review checks the combined diff, scope, test coverage and behaviour, without rerunning the commands. With `judges=`, the judges replace Verify.
   5. When both pass, the cards become Done/verified. With `verify=off`, the batch still writes tests, skips the check and marks the cards accepted; it does not claim they passed.

   Scope is each card's `Files:` and `Tests:` plus its fallout: edits elsewhere that only keep the project consistent with the card's change (another use of a name it renamed, removed or re-typed, a regenerated golden, a registry entry for a surface it added, removed or re-typed), never new behaviour or a weaker test. The code phase makes them, except in a file a sibling card has dirty in a shared worktree, which the batch worker fixes; review accepts them, so a contract change does not fail a batch on scope alone.

   A rejected batch goes back to its batch worker with all reasons. It repairs code or tests within the cards' combined scope, commits, and checks again, up to `attempts`. It does not replay every implementation or cherry-pick integrated commits again. A batch still failing after those attempts (or missing, or inconclusive) does not end the run: its cards stay In progress, the round closes, and the next round repairs that batch first, with every reason, before any other wave. Only when rounds run out are its cards parked as unverified in Backlog, their changes kept on the run branch and named in the report. Blocked or stuck batch leads keep those outcomes. Checks grade every card: a card that passed goes to Done/verified even when the batch failed, unless it shares a file with a failing card, and the repair checks only the failing cards (the verified ones may not change).

   Both `isolation=ticket` and `isolation=shared` use this gate, including a one-card wave. Reading and research cards keep their Work → Verify flow. With `isolation=ticket` the next wave codes in its own worktrees while a batch is being validated, and a card whose dependency is in that batch can start then; its integration waits for the batch. With `isolation=shared` no later wave starts while a batch is being validated.
5. **Branch review.** Once the queue is empty, one reviewer checks the whole diff for missing work. It reuses the latest verified batch's suite/lint evidence when the commit and commands still match, otherwise it runs them. Its findings become cards for the next round. A defect in code the run committed is always a fix card with regression coverage, never a loose end.
6. **Finish.** By default the branch and worktree stay as they are, without a question. With `approve=on`, you pick one: merge into your branch, push and open a PR, open a PR and merge when CI is green, or keep the branch. To discard the branch, type `Discard` (the card holds four options).
7. **Land.** After either PR option, Deepwork watches the PR's checks (`gh pr checks --watch`, as a background job, for up to an hour). On green, the merge option squash-merges the PR and deletes the remote branch, and the plain option comments `CI green` on the run ticket. A repository with no checks counts as green. If a check fails and rounds remain, Deepwork reads the failed log, makes one `Fix CI: <check>` ticket, runs a round for it, pushes the fix and watches again. Otherwise it reports `CI: <state>, PR open` and keeps the worktree. It needs `gh`, signed in.

Sub-agents work only inside their worktree. They never push, merge, rebase, reset, switch, stash or clean. Deepwork pushes or merges only when you pick that at the finish, and a run never deploys or writes to production unless your latest message asks for it. Loose ends the goal doesn't need are not tickets: they go in the report and, when any remain, in one `Follow-ups: <goal>` subtask in Backlog labelled `pending`, a checklist from which you promote a line to its own ticket.

## The board

- **Comments.** Tests, implementation, verification and review return state results without ticket comments. The orchestrator posts one consolidated terminal handoff per ticket with outcome, verification, evidence reference and landed SHA. Earlier comments are reserved for new actionable blockers, human questions or material decisions. Tool calls, successful checks and internal retries stay in traces/state; unchanged facts are deduplicated. Round checkpoints and the final report remain on the run ticket for resume and audit.
- **Run ticket.** It is titled `deepwork: <goal>` and labelled `dw:run`. Every ticket of the run is its subtask, so its page shows the run's progress. Each round adds a comment with the cost, written as a lower bound (`≥ $1.84`) and, with a harness that reports it, naming a model without pricing (`incomplete (<provider/model> unpriced)`).
- **Queue.** The subtasks in To do are the queue, in board order. In progress means the ticket is being implemented or its integrated batch is being validated. The session tree shows each lead as `<KEY> · <title>`, with its phases under it.
- **Route a ticket.** Set its assignee to `deepwork-worker*` or `deepwork-impl*`. With any other assignee, a ticket that changes code or files runs as `deepwork-impl`, and any other ticket runs as `deepwork-worker`. To add your own tier, create `agents/deepwork-impl-<name>.md` with `extends: deepwork-impl` (or `deepwork-worker`) and a `model`.
- **Change the plan.** Tell Deepwork in the chat. It edits the queued subtasks from the next round. Running tickets finish first, then each one gets a follow-up.
- **Steer between rounds.** You can:
  - add a subtask on the run ticket and move it to To do;
  - reorder To do;
  - comment on a ticket to steer its retry;
  - move a Backlog card to To do to run it again.
- **Resume a run.** If the orchestrator's session is lost (the console closed it, the engine restarted), start Deepwork again with `args: resume=<R>`. It adopts the run ticket: the goal, args, branch and worktree come from the card, a batch still being validated is adopted and awaited, never stopped (a stop kills its suites with SIGKILL, so their restores would not run), a batch that was not verified is repaired first (its cards are already on the run branch, so their code is not redone), In progress tickets whose lead is gone and `unfinished` Backlog tickets go back to To do and build on what was already committed, and the rounds continue from where they stopped; `max_rounds` then counts the rounds this resume runs. A finished run resumes the same way, as long as its worktree still exists; only failed, blocked or stuck cards need you to move them to To do.
- **Skip a ticket.** Move it from To do to Backlog. Don't drag In progress cards, because Deepwork's next write overrides your move. Deleting a card removes it from the run.
- **Outcomes.**
  - Done tickets carry `verified` or `accepted`.
  - Backlog tickets carry `blocked`, `failed`, `unverified`, `unfinished`, `stuck` or `pending`.
  - Tickets dropped from the plan are `canceled` and go to the board's canceled column. Older boards may lack one: add it in the Kanban settings. Otherwise those tickets go to Backlog with the `canceled` label.

  Deepwork only changes these outcome labels, so your own labels stay.

## Prerequisites

- **kanban** with the default columns: ids `backlog`, `todo`, `in_progress` and `done`, and `done_status: done`.
- **state** with the defaults: the kv adapter (redis won't work), `store_method: file_based` and `triggers_enabled: true`.
- **harness** with `max_depth` of at least 3 and `max_children` of at least `concurrency`. The defaults, 3 and 8, are enough.
- **Approval rules**, only when the project runs the `approval-gate` worker (a project without it prompts for nothing, so skip this). These functions must run without a prompt:

  | Who | Functions |
  |---|---|
  | the harness | `state::barrier` |
  | orchestrator | `harness::spawn`, `harness::metrics`, `harness::stop`, `state::compare-and-set`, `state::get`, `kanban::ticket::list`, `kanban::project::create`, `shell::exec`, `shell::exec_bg`, `shell::status`, `shell::kill`, plus `directory::agents::get` with `judges=` |
  | leads | `harness::spawn`, `harness::status`, `harness::stop`, `engine::register_trigger`, `engine::unregister_trigger`, `harness::triggers::list`, `state::list`, `state::compare-and-set` |
  | phase sub-agents | `state::compare-and-set`, `kanban::ticket::get`, `shell::exec`, `shell::exec_bg`, `shell::status`, `shell::kill`, `engine::register_trigger` (long runs), `coder::read-file`, plus whatever the work needs. `deepwork-impl` also needs `coder::create-file` and `coder::update-file`. Checkers, and the worker of the plain flow's synthesis ticket, also need `state::get` |

  A phase call that waits for approval parks its ticket as `blocked`. If every lead in a wave waits for approval, the run ends as `blocked`.
- **`harness::stop`.** With `approval-gate`, the repository rules (`iii-permissions.yaml`) may deny it with a `'!harness::stop'` line. To allow it, remove that line and add `- harness::stop`. An allow above the deny line is not enough, because the console makes every deny rule session-wide. If you keep the deny, leads that time out are parked as `stuck`.
- **Project creation.** The repository rules allow a `kanban::project::create` whose `lead` is `deepwork` (rule `deepwork-run-project`). With older rules, add that rule or pass `project=`. If the create is refused, the run goes on without a project.
- **Trust.** Any agent that can create or update kanban tickets can add a subtask to the run ticket. That subtask then runs with the orchestrator's permissions.

## How it works

1. Each round takes the run ticket's To do subtasks and runs them in waves of `concurrency` leads.
2. For each wave, the orchestrator moves the tickets to In progress and spawns one `deepwork-lead` per ticket. It then waits on one `state::barrier` wake with a deadline.
3. A dev code lead runs just Code and returns its implementation result. After integration, one batch lead runs Work (tests/repairs), then Check (Verify and Review in parallel), and retries the batch on rejection. Plain ticket leads keep Work and Check. Every lead returns one result to state.
4. The orchestrator wakes when every implementation lead has written or the deadline passes, then again when the batch lead returns or times out. With a harness that tells a parked parent when a child ends, a lead or the orchestrator also wakes when a child ends without a result and fails that attempt at once, instead of waiting out the deadline; older harnesses send no such notice, so the deadline applies. A lead with no valid result is stopped, and its ticket is retried next round. If the lead can't be stopped, the ticket is parked as `stuck`. With a harness and ide that reap background jobs on stop, the stop also kills the host `shell::exec_bg` jobs of the stopped lead and its phases (a sandbox job is only marked killed; its in-VM process runs until its `timeout_ms` or `sandbox::stop`); with older ones those jobs keep running.
5. Dev code tickets wait in In progress for the batch gate. Verified and accepted tickets then go to Done. Failed implementations go back to To do while rounds remain; a failed batch is repaired first in the next round, its cards still In progress, and parks them in Backlog as unverified only when rounds run out. Their code is never replayed.
6. The run ends when one of these happens:
   - rounds run out;
   - nothing is left;
   - progress stalls;
   - the budget is spent;
   - approvals block it;
   - you stop it.

   The report goes on the run ticket, into the chat and to `notify`.

A command that outlives the foreground cap (`shell::exec` stops at 120 s), such as the baseline, the full suite or the first build in a fresh ticket worktree, runs as a background job (`shell::exec_bg`), in any phase. The agent registers a `shell::job-finished` wake for that job and ends its turn; the harness wakes it with the exit code when the job ends. It never polls, and it kills a job whose wake expires. This needs `ide` 0.12.19 or later.

Results go through `state` rather than kanban events. State has compare-and-set, and a kanban outage won't lose results.

Not yet: a hard cost cap from the console.
