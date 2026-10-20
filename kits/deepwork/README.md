# deepwork

Give **Deepwork** a goal and a team of agents works it, with the kanban board as the task list:

1. Deepwork breaks the goal into tickets.
2. Each ticket gets a lead agent that takes it through its phases with its own sub-agents.
3. Leads run in parallel, in waves.
4. Rounds repeat until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

A goal that changes code in a git repository runs the **dev flow**. You approve the design and the plan. Each code ticket is written test-first in its own git worktree, with one commit. Every commit is verified and reviewed (with `verify=off`, accepted as written), then landed on the run branch. The whole branch is reviewed, and you choose how it finishes: merge it, open a PR, or open a PR that merges once CI is green. Other goals (research, reading, analysis) run the **plain flow**: you approve the plan and each ticket is worked and verified. When at least two tickets finish with a result, and the run was not stopped, blocked or over budget, one last ticket merges the results into a single verified report that cites the ticket behind every claim (`synthesize=off` turns it off).

Deepwork needs only the `harness`, `state`, `kanban` and `ide` workers.

## Profiles

| Profile | What it does | Model to pin |
|---|---|---|
| `deepwork` | the orchestrator you talk to | none: uses the session's |
| `deepwork-lead` | runs one ticket through its phases | a fast, cheap one |
| `deepwork-worker` | research, reading, analysis | optional |
| `deepwork-impl` | writes the failing test and the code, and commits | a strong one |
| `deepwork-adversary` | verifies and reviews each ticket (the default checker) | a mid-tier one |
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

- `agents/deepwork-lead.md`
- `agents/deepwork-impl.md`
- `agents/deepwork-adversary.md`
- `agents/deepwork-worker.md` (optional)

Use the `provider::model` form so `reasoning_effort` reaches the provider. Kit updates keep your edits.

A tier without its own model uses the one set on Default (`agents/default.md`). If Default has none, it uses the orchestrator's model. An unpinned lead spends one turn per phase on that model. With `judges=`, `deepwork-verifier` must resolve no model at all, Default included, or the run won't start.

## Run it

In the console, pick **Deepwork** and type the goal. To change the defaults, add an `args:` line:

```
Add per-user rate limits to the login route.
args: max_rounds=2 concurrency=4 budget_usd=5
```

| Arg | Default | What it does |
|---|---|---|
| `flow` | `auto` | `dev` for code goals in a git repository, `plain` for the rest. Set it to force one |
| `approve` | on | waits for you on open design questions and on the plan, and in the dev flow on the finish. `off` runs straight through and keeps the branch |
| `synthesize` | on | in the plain flow, ends the run with one ticket that merges the done tickets' results into a single report, verified like any other ticket. It runs when at least two tickets are done, and not after a stop, a block or the budget. `off` reports per ticket |
| `plan_only` | off | `on` plans and stops: the run ticket and its cards go on the board, nothing runs. Edit the cards, then start with `resume=<R>` |
| `isolation` | `ticket` | in the dev flow, each code ticket works in its own worktree, branched from the run branch and landed on it when verified. `shared` runs every ticket in the run worktree, so tickets that share a file or a test target wait for a later wave |
| `max_rounds` | 3 | rounds over the queue |
| `attempts` | 2 | tries per ticket per round. A rejected try is retried with the reasons |
| `concurrency` | 8 | leads per wave, at most 8. A ticket waits for its `Depends on:` tickets. With `isolation=shared`, tickets that share a file never share a wave |
| `verify` | on | checks each ticket |
| `verifier` | `deepwork-adversary` | the checker profile: `deepwork-adversary*` or `deepwork-verifier*` |
| `judges` | – | comma-separated model ids. One `deepwork-verifier` per model checks each ticket and the majority wins. At most 7 in the dev flow, 8 in the plain flow |
| `deadline_min` | 20 | minutes each phase gets |
| `critic` | on | one last check for anything missing once the queue is empty. In the dev flow, this is the branch review |
| `max_items` | 5 × `concurrency` | the most tickets a run creates |
| `budget_usd` | – | soft cost cap. It is checked before each wave, and no new wave starts once it is reached. A model without catalog pricing, or a call without usage, isn't counted, so the cap can be overshot |
| `notify` | – | a function id that gets the final report, such as a Slack or email send |
| `project` | – | an existing kanban project (id or name) for the run. Without it, each run creates its own project, `deepwork: <goal> · <id>` |
| `release` | – | an existing kanban release (id or name). Agents can't create releases |
| `resume` | – | the key of an earlier run ticket to pick up from the board instead of planning anew (see Resume a run) |

### Hard cost cap

The console only has the soft `budget_usd`. For a hard cap, start the run from the CLI:

```
iii trigger harness::send --json '{"message":"<goal + args>","model":"<provider::model>","options":{"agent":"deepwork","max_cost_usd":5,"max_total_tokens":2000000}}'
```

You must pass `model`, because the profile sets none. Every model the run uses needs catalog pricing, judges included. The design, plan and finish questions show up in that session in the console. Pass `approve=off` to skip them.

## The dev flow

1. **Design.** Deepwork reads the repository and asks about anything the code can't answer. It asks one question at a time and offers two or three options, with its own pick first.
2. **Plan.** The design goes on the run ticket. Each task becomes a subtask with `Files:`, `Test first:`, `Implement:`, `Verify:` and `Commit:` lines. A ticket you named, or that the run ticket mentions, becomes a subtask of the run ticket with its card appended instead of a copy; a goal that is itself a ticket key makes that ticket the run ticket. When the goal needs a browser and the browser worker reports a problem, the card carries a `preflight: browser: …` line; the run goes on. You answer one card: Start, Stop, or type your changes. If you type changes, Deepwork rewrites the plan and asks again.
3. **Worktree.** The run gets a new branch, `deepwork/<slug>`, in `.worktrees/<slug>`. That folder is excluded locally, so your checkout is untouched. Run Deepwork from the repository's top folder; if the files the goal names are outside it, Deepwork stops and names the folder to start from. The test suite runs once, as a background job, to get a baseline. Each code ticket then works in its own worktree, `.worktrees/<slug>-<key>`, on a branch off the run branch. A fresh copy may need an install step before it builds (a Node project): Deepwork reads it from the repository and runs it in each ticket worktree. Build output is never shared between worktrees, so a compiled project builds once per ticket.
4. **Code tickets.** Each code ticket follows four steps:
   1. Write a failing test.
   2. Write the least code that makes it pass, and commit once.
   3. Verify and review the commit, side by side. Verify re-runs the `Verify:` lines; a run too long for the foreground cap goes to a background job. Review checks that the tests are unchanged and nothing outside `Files:` moved, and that the change is correct: for each new branch, state or caller it names an input that would break it and reads the code that handles it. A confirmed defect is a rejection with the failing input as its reason; when no test would catch it, the reason starts with `test:` and the tests step runs again first.
   4. Land the commit on the run branch. A conflict with a sibling is a rejection, retried next round on the new base.

   After a wave lands, the full test suite runs once on the run branch as a background job, since each ticket was verified alone. A failure the baseline did not have becomes a `Fix:` ticket, worked like any other, with one extra round when the run has none left. The suite is skipped when it would test exactly what a ticket's verify already tested: one ticket landed and the branch's files match its worktree.

   A rejection goes back to the same code session with the reasons. Research tickets in the same run are worked and verified without a commit.
5. **Branch review.** Once the queue is empty, one reviewer runs the full test suite and the linter, each as one background job, and reviews the whole diff. Its findings become tickets for the next round. A defect in code the run committed is always a fix ticket whose test is the input that shows it, never a loose end.
6. **Finish.** You pick one: merge into your branch, push and open a PR, open a PR and merge when CI is green, or keep the branch. To discard the branch, type `Discard` (the card holds four options).
7. **Land.** After either PR option, Deepwork watches the PR's checks (`gh pr checks --watch`, as a background job, for up to an hour). On green, the merge option squash-merges the PR and deletes the remote branch, and the plain option comments `CI green` on the run ticket. A repository with no checks counts as green. If a check fails and rounds remain, Deepwork reads the failed log, makes one `Fix CI: <check>` ticket, runs a round for it, pushes the fix and watches again. Otherwise it reports `CI: <state>, PR open` and keeps the worktree. It needs `gh`, signed in.

Sub-agents work only inside their worktree. They never push, merge, rebase, reset, switch, stash or clean. Deepwork pushes or merges only when you pick that at the finish, and a run never deploys or writes to production unless your latest message asks for it. Loose ends the goal doesn't need are not tickets: they go in the report and, when any remain, in one `Follow-ups: <goal>` subtask in Backlog labelled `pending`, a checklist from which you promote a line to its own ticket.

## The board

- **Comments.** Tests, implementation, verification and review return state results without ticket comments. The orchestrator posts one consolidated terminal handoff per ticket with outcome, verification, evidence reference and landed SHA. Earlier comments are reserved for new actionable blockers, human questions or material decisions. Tool calls, successful checks and internal retries stay in traces/state; unchanged facts are deduplicated. Round checkpoints and the final report remain on the run ticket for resume and audit.
- **Run ticket.** It is titled `deepwork: <goal>` and labelled `dw:run`. Every ticket of the run is its subtask, so its page shows the run's progress. Each round adds a comment with the cost, written as a lower bound (`≥ $1.84`) and, with a harness that reports it, naming a model without pricing (`incomplete (<provider/model> unpriced)`).
- **Queue.** The subtasks in To do are the queue, in board order. In progress means a lead is on the ticket. The session tree shows each lead as `<KEY> · <title>`, with its phases under it.
- **Route a ticket.** Set its assignee to `deepwork-worker*` or `deepwork-impl*`. With any other assignee, a ticket that changes code or files runs as `deepwork-impl`, and any other ticket runs as `deepwork-worker`. To add your own tier, create `agents/deepwork-impl-<name>.md` with `extends: deepwork-impl` (or `deepwork-worker`) and a `model`.
- **Change the plan.** Tell Deepwork in the chat. It edits the queued subtasks from the next round. Running tickets finish first, then each one gets a follow-up.
- **Steer between rounds.** You can:
  - add a subtask on the run ticket and move it to To do;
  - reorder To do;
  - comment on a ticket to steer its retry;
  - move a Backlog card to To do to run it again.
- **Resume a run.** If the orchestrator's session is lost (the console closed it, the engine restarted), start Deepwork again with `args: resume=<R>`. It adopts the run ticket: the goal, args, branch and worktree come from the card, In progress tickets whose lead is gone go back to To do, and the rounds continue from where they stopped; `max_rounds` then counts the rounds this resume runs. A finished run can be resumed the same way once you move cards to To do, as long as its worktree still exists.
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
3. Each lead does the same for its phases: one sub-agent per phase and one barrier to wait on. It retries rejected attempts, then writes one result to state.
4. The orchestrator wakes once, when every lead has written or the deadline passes. With a harness that tells a parked parent when a child ends, a lead or the orchestrator also wakes when a child ends without a result and fails that attempt at once, instead of waiting out the deadline; older harnesses send no such notice, so the deadline applies. A lead with no valid result is stopped, and its ticket is retried next round. If the lead can't be stopped, the ticket is parked as `stuck`. With a harness and ide that reap background jobs on stop, the stop also kills the host `shell::exec_bg` jobs of the stopped lead and its phases (a sandbox job is only marked killed; its in-VM process runs until its `timeout_ms` or `sandbox::stop`); with older ones those jobs keep running.
5. Verified and accepted tickets go to Done. Failed ones go back to To do while rounds remain, and the rest go to Backlog.
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
