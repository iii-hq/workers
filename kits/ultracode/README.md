# ultracode

Claude Code-style dynamic workflows on iii, with the kanban board as the task manager. Give **Ultracode** a goal: it breaks the goal into tickets, gives each ticket a lead that runs it through its phases with its own sub-agents, runs the leads in parallel in bounded waves, and loops in bounded rounds until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

A goal that changes code in a git repository gets the [superpowers](https://github.com/obra/superpowers) flow: you approve the design and the plan, every ticket is built test-first in a git worktree with one commit, an independent verifier and a code reviewer check every commit, the whole branch is reviewed, and you choose how it finishes.

| Profile | Role | Model |
|---|---|---|
| **ultracode** | the orchestrator you talk to; the only writer of ticket fields | yours (the session's) |
| **ultracode-lead** (hidden) | runs one ticket through its phases and reports back | pin a fast, cheap model |
| **ultracode-worker** (hidden) | the work phase: research, reading, analysis | set one, or inherit |
| **ultracode-impl** (hidden) | the tests and code phases: failing test, least code, commit | pin a strong model |
| **ultracode-verifier** (hidden) | judge (one per model), critic and branch review; base of the check tiers | none (nor on Default): judges pick per spawn |
| **ultracode-qa** (hidden) | verify and code review in each check (the default `verifier`) | pin a mid-tier model |

No `workflow` worker, code runner or fp pipeline: just `harness`, `state`, `kanban` and `ide` (the shell and file functions the tiers preload).

## Install

Not in the registry yet: the command below works once `iii-hq/ultracode@0.1.0` is published.

```
iii trigger directory::download-kit kit=iii-hq/ultracode apply=true --timeout-ms 1200000
```

`apply=true` installs at once when the plan has no warnings, blocks or pending decisions (for example, a profile with the same id already exists). Otherwise the reply is a plan: review and apply it in the ADE (Directory → Kits), or run `iii trigger directory::kits::apply plan_id=<plan_id from the reply> --timeout-ms 1200000`. Applying adds or updates workers through Compose and can take minutes; if the command still times out, the apply keeps running: check Directory → Kits instead of re-running it.

Once it is applied, pin the models of the tiers. The kit ships them without `model` because it cannot know your providers. Edit `agents/ultracode-lead.md`, `agents/ultracode-impl.md` and `agents/ultracode-qa.md` (and `ultracode-worker.md` if you like) and add `model: <provider::model>`, e.g. `model: anthropic::claude-opus-5-5`. Use the `provider::model` form: it pins the provider and lets `reasoning_effort` reach it. An unpinned lead runs on the orchestrator's model, and it takes a turn per phase. A tier without its own `model` inherits one set on Default (`agents/default.md`, which the Directory editor writes), and that beats both the orchestrator's model and the judge models; with `judges=` the run refuses to start if `ultracode-verifier` resolves a model. Editing an installed kit file is fine; a kit update merges your edits.

## Run it

**From the console:** pick **Ultracode** and type the goal, optionally followed by an `args:` line:
```
Add per-user rate limits to the login route.
args: max_rounds=2 concurrency=4 budget_usd=5
```

| Arg | Default | Meaning |
|---|---|---|
| `flow` | `auto` | `dev` (the superpowers flow) when the goal changes code or files in a git repository, else `plain`; set `dev` or `plain` to force one |
| `approve` | on | the dev flow waits for you on the design, the plan and the finish; `off` runs straight through and keeps the branch |
| `max_rounds` | 3 | retry cycles over the queue |
| `attempts` | 2 | code (or work) attempts a lead makes per round; a rejected attempt goes back to the same code session with the reasons |
| `concurrency` | 4 | leads per wave (max 8) |
| `verify` | on | run each ticket's check step |
| `verifier` | `ultracode-qa` | tier for verify and review (`ultracode-qa*` or `ultracode-verifier*`) |
| `judges` | – | comma-separated model ids (at most 7 in the dev flow, 8 in the plain one): one `ultracode-verifier` per model in each check, majority wins |
| `deadline_min` | 20 | per-phase deadline: each phase sub-agent gets it from its own start. A lead may run 3 × attempts phases for a dev ticket, 2 × attempts with a check in the plain flow (or without one in the dev flow), attempts for work alone, each with two minutes of slack; its wave closes one minute after that |
| `critic` | on | one completeness critic when the queue runs dry; in the dev flow it is the branch review: full test suite, linter and a review of the whole diff |
| `max_items` | 5 × K | cap on tickets the run creates; K = leads per wave = min(concurrency, 8) |
| `budget_usd` | – | soft cap: checked with `harness::metrics` before each wave and at each round close; no new wave starts once it is reached. When a model has no catalog pricing or usage is missing, the cost is a lower bound (marked incomplete when the harness total is null; one generation without usage is left out silently), so the cap can be overshot |
| `notify` | – | a function id called with the final report (e.g. a Slack or email send) |
| `project` | – | an existing kanban project (id or name) to file the run ticket and every item under |
| `release` | – | an existing kanban release (id or name), the same way. Agents can't create projects or releases: make them in the console first |

**With hard caps:** console runs have only the soft `budget_usd`. For a hard ceiling, start the run from the CLI (or any non-agent client):
```
iii trigger harness::send --json '{"message":"<goal + args>","model":"<provider::model>","options":{"agent":"ultracode","max_cost_usd":5,"max_total_tokens":2000000}}'
```
`model` is required because the profile sets none. Every model used in a capped run, every tier and judge included, needs catalog pricing. The dev flow's design, plan and finish questions appear in that session in the console; pass `approve=off` to run without them.

## The dev flow

1. **Design.** Ultracode reads the repository (`git status`, the tree) and asks you what the code can't settle, one question card at a time, with two or three approaches and its recommendation first.
2. **Plan.** The design goes into the run ticket's description and each task becomes a subtask with a test-first card: `Files:`, `Test first:`, `Implement:`, `Verify:`, `Commit:`. A task is two to five minutes of work. You get one card: Start, Stop, or type your changes. A change rewrites the design on the run ticket first, then the affected subtasks (new ones are added, dropped ones go to Backlog with a comment), and the card comes back.
3. **Worktree.** `git worktree add .worktrees/<slug> -b ultracode/<slug> <your branch>` (the slug ends with part of the session id, so runs never share a branch); run Ultracode from the repository's top folder. `.worktrees/` is excluded locally (`.git/info/exclude`), so your files stay untouched. The test suite runs once for a baseline.
4. **Per ticket**, its lead runs `tests` (write the failing test, see it fail for the right reason), `code` (the least change that passes it, then one commit `<Commit:> (<KEY>)`), then `verify` (re-runs the Verify lines) and `review` (reads the commit, checks the test files are unchanged and nothing outside `Files:` moved) at the same time. A rejection goes back to the same code session with the reasons; a reason about the test reruns the tests phase. Tickets that touch the same file never run in the same wave: they share the worktree.
5. **Branch review.** When the queue runs dry, one reviewer runs the full suite and the linter and reviews `git diff <base>...<branch>`. Findings become tickets for the next round.
6. **Finish.** A card: merge into your branch (then the worktree is removed), push and open a PR, keep the branch, or discard it. Ultracode does only what you pick; it never pushes, merges or deletes on its own.

Sub-agents never push, merge, rebase, reset, switch, stash or clean, and they work inside the worktree: their `filesystem_root` anchors every relative path there, and a stack whose filesystem boundary is `workspace` also refuses writes outside it. Loose ends a sub-agent notices but the goal doesn't need become Backlog subtasks labelled `pending`.

## The board

- The run ticket is titled `ultracode: <goal>` with the label `uc:run`. Every run item is one of its subtasks: its page lists them with a done/total progress bar, and each round adds a summary comment with the cost. Filter the board by the run ticket to see only the run.
- The run's subtasks in To do, in board order, are the queue. In progress means its lead is running; the session tree shows each lead as `<KEY> · <title>` with its phases (`<KEY> tests`, `code`, `verify`, `review`) under it.
- **Route a ticket:** set its assignee to a work tier (`ultracode-worker*` or `ultracode-impl*`). Any other assignee (e.g. kanban's `backend-engineer`) does not run that profile: the ticket runs as `ultracode-impl` when its Deliverable changes code or files, else as `ultracode-worker`, with a comment saying so. A new assignee takes effect on the next attempt that starts after the previous one stopped. Add your own tiers as `agents/ultracode-impl-<name>.md` with `extends: ultracode-impl` (or `ultracode-worker`) and a `model`.
- **Change the plan mid-run:** tell Ultracode in the chat. It rewrites the design on the run ticket, edits the queued subtasks, adds new ones and drops the rest to Backlog, starting next round. Subtasks already In progress keep running; a follow-up subtask aligns each one after it is done.
- **Steer between rounds:** add a subtask on the run ticket's page (it starts in the first column) and drag it to To do, drag To do cards to reorder, comment on a ticket to steer its retry, drag a Backlog card to To do to re-queue it (a `stuck` card runs again only after its lead has stopped; until then it goes back to Backlog).
- **Don't** drag In progress cards: the orchestrator's next write wins. Move a queued card to Backlog to skip it (it is skipped from its wave on). A deleted card, or one detached from the run ticket, leaves the run and is listed as `removed` in the report. Deleting the run ticket only stops writes to it (round comments, report, final move) and new items; the existing items keep moving and the report goes only to the chat and to `notify`.
- Outcomes: Done carries `verified` or `accepted`; Backlog carries `blocked`, `failed`, `unverified`, `unfinished`, `stuck`, or `pending` for a loose end. The orchestrator edits only the outcome labels, so your own labels stay.

## Prerequisites

- kanban's column ids `backlog`, `todo`, `in_progress` and `done`, with `done_status: done` (the default, the last column) so the run ticket's progress counts finished items.
- state with the kv adapter (the redis adapter refuses `state::barrier`), `store_method: file_based` (the default; `in_memory` loses barrier progress and results on restart) and `triggers_enabled: true` (the default).
- harness `max_depth` of at least 3 (the default): orchestrator → lead → phase.
- **Approval gate.** Allow these to run without a prompt:
  - `state::barrier`, unattended: the harness calls it on every state write in a wave's or phase's scope.
  - for the phase sub-agents: `state::compare-and-set`, `kanban::ticket::get`, `kanban::comment::create`, `shell::exec`, `coder::read-file` (plus `coder::create-file` and `coder::update-file` for `ultracode-impl`, and `state::get` for verifiers), and whatever else the work needs. A call held for approval in a phase sub-agent parks its ticket as `blocked` with the function to approve; a wave of leads held on approvals ends the run as `blocked`.
  - for leads: `harness::spawn`, `engine::register_trigger`, `state::list`, `state::compare-and-set`, `harness::status`, `harness::stop`, `kanban::comment::create`.
  - for the orchestrator: `harness::spawn`, `harness::metrics`, `state::compare-and-set`, `kanban::ticket::list`, `shell::exec`, `harness::stop`, and `directory::agents::get` when `judges=` is used. Without a rule, each `harness::stop` of a timed-out lead or phase waits for your approval. The repository rule set (`iii-permissions.yaml`) denies it with `'!harness::stop'` and the first matching rule wins: to allow it, put `- harness::stop` above that line (the allow then applies to every agent). If you keep the deny, timed-out leads are parked as `stuck`.
- **Trust.** Any agent allowed `kanban::ticket::create` or `kanban::ticket::update` can add a subtask to the run ticket, which then runs with the orchestrator's permissions.

## How it works

1. Each round takes a snapshot of the run ticket's To do subtasks (`kanban::ticket::list` with `parent_id`) and runs it in waves of `concurrency` leads.
2. Per wave, the orchestrator arms one `state` wake gated by `state::barrier` with a deadline, moves the tickets to In progress and spawns one `ultracode-lead` per ticket.
3. A lead runs its ticket's phases the same way: per phase, one barrier wake with a deadline and one spawn per key; each phase sub-agent posts one comment and writes one document with set-if-absent `state::compare-and-set`. The lead retries rejected attempts, then writes one result for the orchestrator.
4. The wave's barrier wakes the orchestrator once when every lead has written, or the deadline does. It reads all results with one `state::list`. A missing, malformed or rewritten result is null: the lead is stopped if still running (the stop cascades to its phases, but not to `shell::exec_bg` jobs they started) and the ticket is retried next round in the same lead session while rounds remain, or parked as `stuck` if it cannot be stopped.
5. Verified or accepted tickets go to Done; blocked, unverified and stuck ones to Backlog; failed ones back to To do while rounds remain.
6. The run stops on `max_rounds`, no progress, nothing left (after one critic or branch review), the budget, blocked approvals or your stop. The report goes on the run ticket, into the chat and to `notify`; in the dev flow it then asks how to finish the branch.

**Why fan-in is on state, not on kanban events.** A `kanban:change` wake gated by the same barrier works mechanically, but it counts every change to a ticket (including the orchestrator's own moves) as an arrival, kanban delivers events fire-and-forget with no catch-up, and results kept in comments have no compare-and-set. A kanban outage would lose results instead of delaying them.

**Not in 0.1:** ticket dependencies (rounds order the work), a run-level cost cap from the console, one worktree per ticket (tickets that share a file wait for a later wave instead).
