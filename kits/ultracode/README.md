# ultracode

Claude Code-style dynamic workflows on iii, with the kanban board as the task manager. Give **Ultracode** a goal: it breaks the goal into tickets, routes each ticket to an agent tier, runs them as parallel sub-agents in bounded waves, has independent verifiers try to refute every result, and loops in bounded rounds until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

| Profile | Role | Model |
|---|---|---|
| **ultracode** | the orchestrator you talk to; the only writer of ticket fields | yours (the session's) |
| **ultracode-worker** (hidden) | does one ticket: research, reading, analysis | set one, or inherit |
| **ultracode-impl** (hidden) | does one ticket that changes code or files | pin a strong model |
| **ultracode-verifier** (hidden) | refutes a result; judge (one per model) and completeness critic | none (nor on Default): judges pick per spawn |
| **ultracode-qa** (hidden) | refutes a result in the In review wave (the default verifier) | pin a mid-tier model |

No `workflow` worker, code runner or fp pipeline: just `harness`, `state`, `kanban` and `ide` (the shell and file functions the tiers preload).

## Install

Not in the registry yet: the command below works once `iii-hq/ultracode@0.1.0` is published (it needs the first kanban release with `kanban::ticket::links`).

```
iii trigger directory::download-kit kit=iii-hq/ultracode apply=true --timeout-ms 1200000
```

`apply=true` installs at once when the plan has no warnings, blocks or pending decisions (for example, a profile with the same id already exists). Otherwise the reply is a plan: review and apply it in the ADE (Directory → Kits), or run `iii trigger directory::kits::apply plan_id=<plan_id from the reply> --timeout-ms 1200000`. Applying adds or updates workers through Compose and can take minutes; if the command still times out, the apply keeps running: check Directory → Kits instead of re-running it.

Once it is applied, pin the models of the tiers. The kit ships them without `model` because it cannot know your providers. Edit `agents/ultracode-impl.md` and `agents/ultracode-qa.md` (and `ultracode-worker.md` if you like) and add `model: <provider::model>`, e.g. `model: anthropic::claude-opus-5-5`. Use the `provider::model` form: it pins the provider and lets `reasoning_effort` reach it. A tier without its own `model` inherits one set on Default (`agents/default.md`, which the Directory editor writes), and that beats both the orchestrator's model and the judge models; with `judges=` the run refuses to start if `ultracode-verifier` resolves a model. Editing an installed kit file is fine; a kit update merges your edits.

## Run it

**From the console:** pick **Ultracode** and type the goal, optionally followed by an `args:` line:
```
Audit every route under src/api for missing authorization checks.
args: max_rounds=2 concurrency=4 verifier=ultracode-qa budget_usd=5
```

| Arg | Default | Meaning |
|---|---|---|
| `max_rounds` | 3 | retry and verify cycles |
| `concurrency` | 4 | sub-agents per wave (max 8) |
| `verify` | on | send every `done` result to an independent verifier |
| `verifier` | `ultracode-qa` | verify tier for the In review wave (`ultracode-qa*` or `ultracode-verifier*`) |
| `judges` | – | comma-separated model ids (at most 8): one `ultracode-verifier` per model, majority wins; with J judges a wave holds at most ⌊8/J⌋ tickets |
| `deadline_min` | 20 | per-wave deadline; a still-running straggler is stopped. A work ticket goes back to To do while rounds remain (otherwise Backlog `failed`); a verify straggler is not re-run and is parked as `unverified` |
| `critic` | on | one completeness critic when the queue runs dry |
| `max_items` | 5 × K | cap on tickets the run creates; K = tickets per wave = min(concurrency, 8), or min(concurrency, ⌊8/J⌋) with J judges |
| `budget_usd` | – | soft cap: checked with `harness::metrics` before each work wave and at each round close; no new work wave starts once it is reached. When a model has no catalog pricing or usage is missing, the cost is a lower bound (marked incomplete when the harness total is null; one generation without usage is left out silently), so the cap can be overshot |
| `notify` | – | a function id called with the final report (e.g. a Slack or email send) |

**With hard caps:** console runs have only the soft `budget_usd`. For a hard ceiling, start the run from the CLI (or any non-agent client):
```
iii trigger harness::send --json '{"message":"<goal + args>","model":"<provider::model>","options":{"agent":"ultracode","max_cost_usd":5,"max_total_tokens":2000000}}'
```
`model` is required because the profile sets none. Every model used in a capped run, every tier and judge included, needs catalog pricing.

## The board

- The run ticket is titled `ultracode: <goal>` with the label `uc:run`. Its page lists every run item under linked tickets, and each round adds a summary comment with the cost.
- Run items are titled `[KAN-7] …` (the run key). The To do lane, in board order, is the queue.
- **Route a ticket:** set its assignee to a work tier (`ultracode-worker*` or `ultracode-impl*`). Any other assignee (e.g. kanban's `backend-engineer`) does not run that profile: the ticket runs as `ultracode-impl` when its Deliverable changes code or files, else as `ultracode-worker`, with a comment saying so. A new assignee takes effect on the next attempt that starts after the previous attempt's session has stopped; the session view keeps showing the first tier's name and model. Add your own tiers as `agents/ultracode-impl-<name>.md` with `extends: ultracode-impl` (or `ultracode-worker`) and a `model`.
- **Steer between rounds:** add a `[KAN-7] …` ticket to To do, drag To do cards to reorder, comment on a ticket to steer its retry, drag a Backlog card to To do to re-queue it (a `stuck` card runs again only after its sub-agent session has stopped; until then it goes back to Backlog).
- **Don't** drag In progress or In review cards: the orchestrator's next write wins. Move a queued card to Backlog to skip it (it is skipped from its wave on). A deleted card leaves the run and is listed as `deleted` in the report. Deleting the run ticket only stops writes to it (round comments, report, final move); the items keep moving and the report goes only to the chat and to `notify`.
- Outcomes: Done carries `verified` or `accepted`; Backlog carries `blocked`, `failed`, `unverified`, `unfinished` or `stuck`. The orchestrator edits only these labels, so your own labels stay.

## Prerequisites

- kanban's default column ids: `backlog`, `todo`, `in_progress`, `in_review`, `done`.
- state with the kv adapter (the redis adapter refuses `state::barrier`), `store_method: file_based` (the default; `in_memory` loses barrier progress and results on restart) and `triggers_enabled: true` (the default).
- **Approval gate.** Allow these to run without a prompt:
  - `state::barrier`, unattended: the harness calls it on every state write in a wave's scope.
  - for sub-agents: `state::compare-and-set`, `kanban::ticket::get`, `kanban::comment::create`, `shell::exec`, `coder::read-file` (plus `coder::create-file` and `coder::update-file` for `ultracode-impl`, and `state::get` for verifiers), and whatever else the work needs. A held call in a sub-agent never expires; a wave held on approvals ends the run as `blocked`.
  - for the orchestrator: `harness::spawn`, `harness::metrics`, `state::compare-and-set`, `kanban::ticket::links`, `harness::stop`, and `directory::agents::get` when `judges=` is used. Without a rule, each `harness::stop` of a timed-out child waits for your approval. The repository rule set (`iii-permissions.yaml`) denies it with `'!harness::stop'` and the first matching rule wins: to allow it, put `- harness::stop` above that line (the allow then applies to every agent). If you keep the deny, timed-out children are parked as `stuck`.
- **Trust.** Any agent allowed `kanban::ticket::create` can queue `[KEY]` work, which then runs with the orchestrator's permissions.

## How it works

1. Each round takes a snapshot of the run's To do tickets (`kanban::ticket::links` on the run ticket) and runs it in waves of `concurrency`.
2. Per wave, the orchestrator arms one `state` wake gated by `state::barrier` with a deadline, moves the tickets to In progress and spawns one hidden leaf per ticket, picked by the ticket's assignee.
3. Each leaf reads its ticket, does it, posts one comment and writes one result document with set-if-absent `state::compare-and-set`.
4. The barrier wakes the orchestrator once when every result is in, or the deadline does. It reads all results with one `state::list`. A missing, malformed or rewritten result is null: the child is stopped if still running (a stop ends its turn, not `shell::exec_bg` jobs it started) and the ticket is retried next round in the same session while rounds remain, or parked as `stuck` if it cannot be stopped.
5. `done` results go to a verify wave of fresh verifiers that re-run the ticket's Verify lines. Pass → Done; fail → back to To do with the reasons.
6. The run stops on `max_rounds`, no progress, nothing left (after one critic), the budget, blocked approvals or your stop. The report goes on the run ticket, into the chat and to `notify`.

**Why fan-in is on state, not on kanban events.** A `kanban:change` wake gated by the same barrier works mechanically, but it counts every change to a ticket (including the orchestrator's own moves) as an arrival, kanban delivers events fire-and-forget with no catch-up, and results kept in comments have no compare-and-set. A kanban outage would lose results instead of delaying them.

**Not in 0.1:** ticket dependencies (rounds order the work), a run-level cost cap from the console, a run view beyond the board.
