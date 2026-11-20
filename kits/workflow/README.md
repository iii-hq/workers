# workflow kit

Give the **Workflow** agent a goal and a team of agents works it, with the kanban board as the task list and the `workflow` worker running the agents:

1. The Workflow agent first asks you the decisions only you can make: rounds of one card, up to four questions, each with a recommended answer. Facts it looks up itself, and a goal that leaves nothing open gets no card. Then, unless the goal already names its change, it explores before it plans: one workflow run of explorers, three competing designs, a refuter for each and a planner that writes the plan and the tickets.
2. The tickets become subtasks of one run ticket. Each wave of ready tickets is one workflow run. Every ticket phase is a node of that run, with its own agent profile and model.
3. A check node verifies each result. On a `fail` the run loops the work back with the check's reasons, up to `attempts` passes.
4. Rounds repeat until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

The workflow kit uses the ticket strategy of [`deepwork`](../deepwork/) (run ticket, cards, outcome labels, rounds, a critic pass, a synthesized report) and installs beside it: its profiles, labels (`wf:run`), branches (`workflow/…`), worktrees (`.worktrees/wf-…`) and state scopes (`wf-…`) are its own.

A goal that changes code in a git repository runs the **dev flow**. The baseline checks run once in the run worktree. Each code ticket is implemented in its own git worktree, then integrated on the run branch. One run per integrated wave writes the wave's tests and checks them (verify and review together), looping the tests back on a fail. That check also holds the change to the code rules the repository writes down (`CONTRIBUTING.md`, `CODING_STANDARDS.md`, `AGENTS.md`, a style or conventions file under `docs/`). Cards become Done only after that check passes. When a run lets a defect through, write the rule in one of those files: the next run's implementer follows it and its check enforces it. A critic node reviews the whole branch, and the branch is kept by default; when the repository has a remote, the run ends by asking whether to push the branch and open a pull request. Other goals (research, reading, analysis) run the **plain flow**: each ticket is worked and checked in the same run, every claim in a result cites where it comes from or is marked `(unsourced)`, and one last ticket can merge the results into one report (`synthesize=off` turns it off).

The kit needs the `workflow`, `harness`, `state`, `kanban`, `ide` and `fp` workers. The `workflow` worker must take the flat `workflow::start` request (`nodes` at the top level, each node with its `task`): an older one refuses every run the kit starts. The `browser` worker is optional: with it, a card that changes a web page is checked in a real tab (see [Pages](#pages)).

## How it differs from deepwork

| | deepwork | workflow kit |
|---|---|---|
| A wave | `harness::spawn` of one lead per ticket, one `state::barrier` wake | one `workflow::start` run, one notification |
| Attempts | the lead relays work and check phases | the run's check loop |
| Results | state documents | node outputs (`workflow::status`, `workflow::node-result`) |
| Per-phase model | lead and verify model args | every node: tier pin, `work_model` / `check_model`, or picked by the kit (`models=auto`) |
| Stuck agent | wave deadline, null procedure | node `timeout_ms`, then the run goes on |
| Provider hiccup | the turn fails | a planner or verifier node is continued once, in its own session, a minute later |
| Progress | session tree | session tree, plus `workflow::status` phases and per-node usage |

Not in the workflow kit 0.1: judges, a second critic pass, `isolation=shared`, `plan_only`, a release field, `notify`, and watching CI or merging the pull request it opens. Use deepwork for those.

## Profiles

| Profile | What it does | Model to pin |
|---|---|---|
| `workflow` | the orchestrator you talk to | none: uses the session's |
| `workflow-worker` | research, reading, analysis | optional |
| `workflow-impl` | implements a card, or writes the tests and repairs of a batch | a strong one |
| `workflow-planner` | explores the goal, writes a design proposal, turns the proposals into the plan | a strong one, or `plan_model=` |
| `workflow-verifier` | checks a result or a batch, runs the baseline, refutes designs and review findings, and reviews the branch | a mid-tier one |

Every profile except `workflow` is hidden. A node's model is the first of: its profile's pin, the run's `work_model` / `check_model` / `plan_model`, the model listed for its stage in `agents/workflow-models.md`, the model the kit picks for its role, the orchestrator's model.

`agents/workflow-models.md` is installed with the profiles but is not an agent: it is the kit's model list, and the one kit file that names models. It has one row per provider and a cell of model ids, best first, for each of the plan, check and work stages; the file says which nodes each stage covers. With `models=auto` (the default), a run whose orchestrator is on a listed provider gives each stage the first id of its cell that the model catalog has, and picks nothing outside the row: a stage with no listed id in the catalog runs on the orchestrator's model. The file ships filled in. Edit a cell to change the choice, or add a row for another provider, named as the `provider` field of `router::models::list`.

For a provider with no row the kit picks by role. The explore, work and code nodes and the batch's tests run one size below the orchestrator's model in its provider's current line-up, never on the smallest size. Every check (the baseline, the ticket and batch checks, the critic) and the plan group (design, refute, plan, the review's refuters) stay on the orchestrator's model, so a weaker result always meets a check on the stronger one. The orchestrator ranks the catalog itself and stays on its own model whenever it cannot tell which size is right below its own.

Either way the work model gets one round: a retry, every ticket of the last round and the synthesis run on the check model. A picked model that cannot run is skipped for the rest of the run: only the stages that used it move, to the next id of their cell, else to the orchestrator's model. The run ticket's `Args` records it as `skip_models=<id>`, so a resume keeps it. A design run that loses its design, refute and plan nodes this way runs once more before the orchestrator plans the tickets itself. Nothing else runs again: a baseline started on its own stays `unknown`, and review findings whose refuters were lost go to the Follow-ups ticket. Every new run tries the model again: remove an id your account cannot run from the list, or pass `skip_models=<id>`.

A ticket the plan marks hard runs on the plan model. The planner writes a `Hard` bullet, with the reason in one clause, on the card of a ticket whose work a smaller model is likely to get wrong the first time. The signals are a closed list: the ticket writes state that parallel threads, processes or requests also write; it changes a permission check or handles a secret; it changes a stored format, migrates data or changes what existing callers of a public interface get; it has an invariant to hold, as a parser, a state machine or date or money arithmetic does; in the plain flow, its sources disagree. That ticket's work or code node runs on the plan model, the one that designs and plans the run, in every round, retries included; every other ticket, and the batch's tests, keep the work model. A missed mark costs one retry round; a wrong one costs the price of the plan model. The orchestrator marks the same way when it writes the tickets itself. A profile pin and `work_model` still win.

`models=own` turns all of it off.

## Pages

When the `browser` worker is installed and finds a Chromium, a card that changes what a person sees on a web page gets a page check: the check starts the project's server in the worktree, opens the page in a private tab, types and clicks as the card says, and reads the visible text and the page's console. An uncaught exception on the page fails the card. Research tickets read a page that renders with JavaScript through `browser::dynamic-fetch`. Install it with `iii call compose::add worker=browser`. Without it, or without a Chromium, the run goes on: page criteria are marked `(not checked in a browser)` and the run ticket's Notes say why. A run never downloads a browser.

## Install

```
iii call directory::download-kit kit=iii/workflow apply=true --timeout-ms 1200000
```

A reply that is a plan instead of an install has warnings or decisions: review and apply it in the ADE under Directory → Kits, or with `iii call directory::kits::apply plan_id=<plan_id> --timeout-ms 1200000`.

To pin a model, add `model: <provider>::<model>` to `agents/workflow-impl.md` or `agents/workflow-verifier.md` (optional: `agents/workflow-worker.md`). To change the models a run picks by itself, edit the table in `agents/workflow-models.md`. Kit updates keep your edits.

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
| `approve` | `auto` | `on` adds a Plan card, a Design card when the design run leaves questions, and the merge and discard options of the Finish card; `auto` gets the Finish card only to ask before a push |
| `deadline_min` / `verify_deadline_min` | 20 / 40 | node timeouts of work and check nodes |
| `board` | `on` | `off` runs a small goal without the kanban board: plan, results and report in the chat, no resume |
| `design` | `auto` | explore and design before planning, unless the goal already names its change, where it goes and a small scope; `on` always, `off` plans the tickets directly. The run ticket's `Design` bullet says which and why |
| `critic` / `synthesize` | `on` | the final review / the merged report |
| `models` | `auto` | `auto` takes each stage's model from `agents/workflow-models.md`, and for a provider with no row there runs the work nodes one size below the orchestrator's model, never on its provider's smallest; `own` keeps every node without a pin or a model arg on the orchestrator's model |
| `skip_models` | none | model ids, comma-separated, that `models=auto` never picks; the run adds a picked model that fails, and the run ticket keeps the list for a resume |
| `work_model` / `check_model` | picked by `models` | models of unpinned work and check nodes |
| `plan_model` | `check_model`, else picked by `models` | model of the design, refute and plan nodes, the review's refuters and a hard ticket's work |
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
- A page check uses one tab of the browser worker at a time per node, and the worker keeps few tabs open (`max_sessions`, 4 by default): with more page checks than that running at once, a check may lose its tab and start over once. The card names the server's port, so two runs that check the same project at the same time collide; the second check answers `unsure`.
- A run's end reaches the Workflow agent as a `workflow::on-run-completed` notification: a policy that narrows that session's `options.functions` must allow `harness::observe` and `workflow::on-run-completed`.
- The dev flow keeps the branch unless the Finish answer says otherwise. The kit pushes only on that answer, and only the run branch, never the base: when the repository has a remote, the Finish card offers to push the branch and open a ready pull request into the base with `gh`. Only an answer that names the push or the pull request counts; any other reply keeps the branch. The run branch starts from your local base, so the card says when the push would also publish base commits the remote does not have. Without `gh`, or when it is not signed in, the branch is pushed and you get the command to run. A run started by another session has nobody to answer the card and keeps the branch. The kit never watches CI or merges the pull request.
- A Workflow agent started by another session has nobody to ask: the interview takes its recommendations, and a goal that leaves a consequential decision open (scope, compatibility, security, data loss, external cost) ends `blocked` with its questions. State those decisions in the goal.
