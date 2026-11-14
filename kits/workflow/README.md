# workflow kit

Give the **Workflow** agent a goal and a team of agents works it, with the kanban board as the task list and the `workflow` worker running the agents:

1. Unless the goal already names its change, the Workflow agent explores before it plans: one workflow run of explorers, three competing designs, a refuter for each and a planner that writes the plan and the tickets.
2. The tickets become subtasks of one run ticket. Each wave of ready tickets is one workflow run. Every ticket phase is a node of that run, with its own agent profile and model.
3. A check node verifies each result. On a `fail` the run loops the work back with the check's reasons, up to `attempts` passes.
4. Rounds repeat until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

The workflow kit uses the ticket strategy of [`deepwork`](../deepwork/) (run ticket, cards, outcome labels, rounds, a critic pass, a synthesized report) and installs beside it: its profiles, labels (`wf:run`), branches (`workflow/…`), worktrees (`.worktrees/wf-…`) and state scopes (`wf-…`) are its own.

A goal that changes code in a git repository runs the **dev flow**. The baseline checks run once in the run worktree. Each code ticket is implemented in its own git worktree, then integrated on the run branch. One run per integrated wave writes the wave's tests and checks them (verify and review together), looping the tests back on a fail. Cards become Done only after that check passes. A critic node reviews the whole branch, and the branch is kept by default. Other goals (research, reading, analysis) run the **plain flow**: each ticket is worked and checked in the same run, and one last ticket can merge the results into one report (`synthesize=off` turns it off).

The kit needs the `workflow`, `harness`, `state`, `kanban`, `ide` and `fp` workers. The `workflow` worker must take the flat `workflow::start` request (`nodes` at the top level, each node with its `task`): an older one refuses every run the kit starts.

## How it differs from deepwork

| | deepwork | workflow kit |
|---|---|---|
| A wave | `harness::spawn` of one lead per ticket, one `state::barrier` wake | one `workflow::start` run, one notification |
| Attempts | the lead relays work and check phases | the run's check loop |
| Results | state documents | node outputs (`workflow::status`, `workflow::node-result`) |
| Per-phase model | lead and verify model args | every node: tier pin or `work_model` / `check_model` |
| Stuck agent | wave deadline, null procedure | node `timeout_ms`, then the run goes on |
| Progress | session tree | session tree, plus `workflow::status` phases and per-node usage |

Not in the workflow kit 0.1: judges, a second critic pass, `isolation=shared`, `plan_only`, a release field, `notify`, and the PR and CI finish options. Use deepwork for those.

## Profiles

| Profile | What it does | Model to pin |
|---|---|---|
| `workflow` | the orchestrator you talk to | none: uses the session's |
| `workflow-worker` | research, reading, analysis | optional |
| `workflow-impl` | implements a card, or writes the tests and repairs of a batch | a strong one |
| `workflow-planner` | explores the goal, writes a design proposal, turns the proposals into the plan | a strong one, or `plan_model=` |
| `workflow-verifier` | checks a result or a batch, runs the baseline, refutes designs and review findings, and reviews the branch | a mid-tier one |

Every profile except `workflow` is hidden. Without a pin, a node runs on `work_model` or `check_model`, else on the orchestrator's model.

## Install

```
iii call directory::download-kit kit=iii/workflow apply=true --timeout-ms 1200000
```

A reply that is a plan instead of an install has warnings or decisions: review and apply it in the ADE under Directory → Kits, or with `iii call directory::kits::apply plan_id=<plan_id> --timeout-ms 1200000`.

To pin a model, add `model: <provider>::<model>` to `agents/workflow-impl.md` or `agents/workflow-verifier.md` (optional: `agents/workflow-worker.md`). Kit updates keep your edits.

## Run it

In the console, pick **Workflow** and type the goal. For a code goal, start it from the repository's top folder. To change the defaults, add an `args:` line:

```
Add per-user rate limits to the login route.
args: max_rounds=2 concurrency=4 budget_usd=5
```

| Arg | Default | Meaning |
|---|---|---|
| `max_rounds` | 3 | rounds before it stops |
| `concurrency` | 4 | tickets per wave (at most 8) |
| `attempts` | 2 | passes of a ticket's work inside one run |
| `flow` | `auto` | `dev` or `plain` to force one |
| `approve` | `auto` | `on` adds Plan and Finish cards, and a Design card when the design run leaves questions |
| `deadline_min` / `verify_deadline_min` | 20 / 40 | node timeouts of work and check nodes |
| `board` | `on` | `off` runs a small goal without the kanban board: plan, results and report in the chat, no resume |
| `design` | `auto` | explore and design before planning, unless the goal already names its change, where it goes and a small scope; `on` always, `off` plans the tickets directly. The run ticket's `Design` bullet says which and why |
| `critic` / `synthesize` | `on` | the final review / the merged report |
| `work_model` / `check_model` | the orchestrator's | models of unpinned work and check nodes |
| `plan_model` | `check_model` | model of the design, refute and plan nodes |
| `budget_usd` | none | stop dispatching once the run's cost reaches it |
| `max_items` | 2 × concurrency, at least 4 | tickets the run may create in all, new items included; the rest go to a Follow-ups ticket |
| `min_free_gb` | 5 | free disk the dev flow needs before it starts or tests |
| `project` | a new one | kanban project to file the run under |
| `resume` | none | an earlier run ticket to adopt |

Every run's nodes carry a phase, in the order the run goes: `baseline`, `explore`, `design`, `refute`, `plan`, `implement` (plain flow: `work`), `verify`, `review`. The ADE's Workflow section lists them per run.

Watch the run on the board, or open the run ticket's waves with `workflow::status {"run_id":"…","details":true}`: phases, labels, the model and usage of every node.

## Steer it

- Add a subtask of the run ticket in To do: it runs next round.
- Comment on a ticket: its next attempt reads the comment.
- Drag a Backlog card to To do to re-queue it. A card in a running wave stays in To do; the ADE's Workflow section shows what is running.
- Type `stop` to abort, `continue` to pick up after an interruption, or start a new session with `args: resume=<run ticket>`.

## Limits

- One `workflow` worker per namespace.
- A node runs as its own session: a session's hard cost cap and its own `options.functions` don't reach the nodes. Every node gets `*` minus the kit's deny list (no board writes, no state writes, no sub-agents, no `workflow::*`). Use `budget_usd` to bound a run.
- A node timeout kills that node only; the run reads it as a failed attempt.
- A run's end reaches the Workflow agent as a `workflow::on-run-completed` notification: a policy that narrows that session's `options.functions` must allow `harness::observe` and `workflow::on-run-completed`.
- The dev flow keeps the branch unless `approve=on` and the Finish answer says otherwise. The kit never pushes.
