# workflow kit

Give the **Workflow** agent a goal and it finishes it the cheapest way that is still right. The `workflow` worker runs the agents of a larger goal, and the kanban board is its task list:

1. The Workflow agent sizes the goal first, with a few read-only calls, and says which path it took. A small one (a question, a lookup, a run-and-report, or a change of at most five files with one obvious way to do it, low stakes and a quick check) it does itself, in your checkout, with no ticket and no workflow run, and reports what it changed and what it ran.
2. Any other goal is a run, with its tickets as subtasks of one run ticket on the board. Before it plans, the agent asks you the decisions only you can make: rounds of one card, up to four questions, each with a recommended answer. Facts it looks up itself, and a goal that leaves nothing open gets no card. One or two tickets run **focused**: with no design run unless the agent could not find the place (then explorers and a planner), one workflow run per ticket, in which one agent writes the code and its tests and a check node verifies them.
3. Anything larger runs **full**. When the agent cannot write the tickets from what it read, it explores first: one workflow run of explorers and a planner, plus three competing designs and a refuter for each only when the approach is a real choice that is costly to reverse, or the goal asks for a design. Each wave of ready tickets is one workflow run. Every ticket phase is a node of that run, with its own agent profile and model.
4. A check node verifies each result. On a `fail` the run loops the work back with the check's reasons, up to `attempts` passes. A pass that fixed none of them ends the loop as `stalled`: the batch gets one repair on the plan model with a root-cause brief, and a second stall parks its cards for you.
5. Rounds repeat until the goal is done, nothing is left, progress stalls, rounds run out or the budget is spent.

The workflow kit uses the ticket strategy of [`deepwork`](../deepwork/) (run ticket, cards, outcome labels, rounds, a critic pass, a synthesized report) and installs beside it: its profiles, labels (`wf:run`), branches (`workflow/…`), worktrees (`.worktrees/wf-…`) and state scopes (`wf-…`) are its own.

A goal that changes code in a git repository runs the **dev flow**. A ticket that runs alone is one run in the run worktree: an implementer node writes the code and its tests, and the check (verify and review together) loops it back on a fail. A wave of several tickets gives each its own git worktree, integrates them on the run branch, and one run writes the wave's tests and checks them. A full run first runs the baseline checks once, in the run worktree; a focused run runs none, and a failing command whose file and imports the change did not touch is reported, not charged to the ticket. A card gets tests only when a regression test can observe its change: a change of looks, wording or a configuration value is proved by its Verify calls. That check also holds the change to the code rules the repository writes down (a markdown file in its top folder or in `docs/`: `CONTRIBUTING.md`, `AGENTS.md`, a UI, style or conventions file). Cards become Done only after that check passes. When a run lets a defect through, write the rule in one of those files: the next run's implementer follows it and its check enforces it. A critic node reviews the whole branch when two or more tickets were verified, a ticket was marked hard or the goal asks for a review (`critic=on`: always), and the branch is kept by default; when the repository has a remote, the run ends by asking whether to push the branch and open a pull request. Other goals (research, reading, analysis) run the **plain flow**: each ticket is worked and checked in the same run, every claim in a result cites where it comes from or is marked `(unsourced)`, and one last ticket can merge the results into one report (`synthesize=off` turns it off).

The kit needs the `workflow`, `harness`, `state`, `kanban`, `ide` and `fp` workers. The `workflow` worker must take the flat `workflow::start` request (`nodes` at the top level, each node with its `task`): an older one refuses every run the kit starts. The `browser` worker is optional: with it, a card that changes a web page is checked in a real tab (see [Pages](#pages)). The `judge` worker is optional too: with it, a run on the board is filed under the project its goal fits (see [Projects](#projects)).

## Paths

The agent says which path it took, and why, in its first line (`Path: focused (one ticket, the files are known)`).

| | direct | focused | full |
|---|---|---|---|
| For | a question, a lookup, a run-and-report, a change of at most five files with one way to do it, low stakes and a quick check | one or two tickets; a place the agent could not find is explored first | everything else: three or more tickets, an audit, a migration, a goal that asks for a design |
| Who works | the Workflow agent | nodes of a workflow run | nodes of workflow runs |
| Board | never | on (`board=off` turns it off) | on (`board=off` turns it off) |
| Design run | none | none, or explorers and a planner when the place is not found | none, explorers and a planner, or the panel of competing designs and refuters |
| Check | the narrow command the agent runs itself | a check node per run | a check node per run, and a branch review when two or more tickets were verified |
| Where it writes | your checkout, uncommitted | a run branch in `.worktrees/` | a run branch in `.worktrees/` |

The goal's own words steer it: one that asks for tickets, the board, a plan, a workflow run or an independent check is never done directly, and `board=on`, `design=on`, `critic=on` or `approve=on` in the `args:` line forces that part (and a run); `project=`, or a goal that names a kanban ticket, is always a run on the board. A path only grows: before the agent wrote anything it just continues as a run, and after a direct edit that did not work out it stops and reports what is left. After a finished code run, a small follow-up to what the branch delivered is done directly, committed on that branch.

## How it differs from deepwork

| | deepwork | workflow kit |
|---|---|---|
| A small goal | a run like any other | done by the Workflow agent itself, with no ticket and no run |
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
| `workflow` | the orchestrator you talk to; does small goals itself | none: uses the session's |
| `workflow-worker` | research, reading, analysis | optional |
| `workflow-impl` | implements a card, implements and tests a card that runs alone, or writes the tests and repairs of a batch | a strong one |
| `workflow-planner` | explores the goal, writes a design proposal, turns the findings or the proposals into the plan | a strong one, or `plan_model=` |
| `workflow-verifier` | checks a result or a batch, runs the baseline, refutes designs and review findings, and reviews the branch | a mid-tier one |

Every profile except `workflow` is hidden. A node's model is the first of: its profile's pin, the run's `work_model` / `check_model` / `plan_model`, the model listed for its stage in `agents/workflow-models.md`, the model the kit picks for its role, the orchestrator's model.

`agents/workflow-models.md` is installed with the profiles but is not an agent: it is the kit's model list, and the one kit file that names models. It has one row per provider and a cell of model ids, best first, for each of the plan, check, work and code stages; the file says which nodes each stage covers. With `models=auto` (the default), a run whose orchestrator is on a listed provider gives each stage the first id of its cell that the model catalog has, and picks nothing outside the row: a stage with no listed id in the catalog runs on the orchestrator's model. The file ships filled in. Edit a cell to change the choice, or add a row for another provider, named as the `provider` field of `router::models::list`.

For a provider with no row the kit picks by role. The explore nodes, the baseline and the plain flow's work nodes run one size below the orchestrator's model in its provider's current line-up, never on the smallest size. Every node that changes files (code, solo, the batch's tests and repairs), every check (the ticket and batch checks, the critic) and the plan group (design, refute, plan, the review's refuters) stay on the orchestrator's model, so a weaker result always meets a check on the stronger one, and no file is written by the smaller model. The orchestrator ranks the catalog itself and stays on its own model whenever it cannot tell which size is right below its own.

Either way the work model gets one round: a retry, every ticket of the last round and the synthesis run on the check model. A picked model that cannot run is skipped for the rest of the run: only the stages that used it move, to the next id of their cell, else to the orchestrator's model. The run ticket's `Args` records it as `skip_models=<id>`, so a resume keeps it. A design run that loses its design, refute and plan nodes this way runs once more before the orchestrator plans the tickets itself. Nothing else runs again: a baseline started on its own stays `unknown`, and review findings whose refuters were lost go to the Follow-ups ticket. Every new run tries the model again: remove an id your account cannot run from the list, or pass `skip_models=<id>`.

A ticket the plan marks hard runs on the plan model. The planner writes a `Hard` bullet, with the reason in one clause, on the card of a ticket whose work a smaller model is likely to get wrong the first time. The signals are a closed list: the ticket writes state that parallel threads, processes or requests also write; it changes a permission check or handles a secret; it changes a stored format, migrates data or changes what existing callers of a public interface get; it has an invariant to hold, as a parser, a state machine or date or money arithmetic does; in the plain flow, its sources disagree. That ticket's work or code node runs on the plan model, the one that designs and plans the run, in every round, retries included; every other ticket, and the batch's tests, keep their stage's model: code for a node that changes files, work for the others. A missed mark costs one retry round; a wrong one costs the price of the plan model. The orchestrator marks the same way when it writes the tickets itself. A profile pin and `work_model` still win. The one repair of a batch whose check stalled runs on the plan model too.

`models=own` turns all of it off.

## Pages

When the `browser` worker is installed and finds a Chromium, a card that changes what a person sees on a web page gets a page check: the check starts the project's server in the worktree, opens the page in a private tab, types and clicks as the card says, and reads the visible text and the page's console. An uncaught exception on the page fails the card. Research tickets read a page that renders with JavaScript through `browser::dynamic-fetch`. Install it with `iii call compose::add worker=browser`. Without it, or without a Chromium, the run goes on: page criteria are marked `(not checked in a browser)` and the run ticket's Notes say why. A run never downloads a browser.

## Projects

A run on the board is filed under a project that stands for a product or a theme, not for the run, so the project list no longer grows by one per run.

- With the `judge` worker and a provider that answers, the agent asks it one question: which active project fits the goal, or none. A confident answer files the run under that project, or under a new one named after the theme. A repository's project is never one of the choices, so a goal no theme covers gets a new theme project. An unsure answer is one more question on a full run's interview card.
- With the `judge` worker installed and no product or theme project on the board yet, there is nothing to ask: the run opens a theme project named after the goal's theme, without a judge call, and the run ticket's `Project` bullet says `(no candidate)`.
- Without the judge, when its call fails, when a focused run gets an unsure answer or when nobody answers the card, the run goes under the repository's project. It is named after the repository's folder (the main checkout, not a worktree) and created once. Its description, `Work in the <name> repository.`, is what marks it: keep that text when you edit the project.
- `project=` wins over all of it. A goal that is a ticket with a project keeps that project, and a resumed run keeps the run ticket's.
- A name that belongs to an archived project is not reused: the run goes without a project and the run ticket's Notes say so. Restore the project on the Projects page, or pass `project=`.
- Projects that older versions named `workflow: …` are never picked. Archive them on the Projects page.

The run ticket's title is the goal, with no `workflow:` prefix; its `wf:run` label marks it as a run, and its `Project` bullet says how the project was chosen.

Creating a project waits for a person unless the session's approval mode or rules allow `kanban::project::create`; when it is refused, the run goes on without a project. A hosted judge provider (`judge-typesafe`, `judge-openai`) receives the first 200 characters of the goal (for a goal that is a ticket: of its title and description), the repository's name and the names and descriptions of the active projects. To keep them on the machine, pick a local judge in the composer or pass `project=`.

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
| `max_rounds` | 3 | rounds before it stops; the review's fixes get one more |
| `concurrency` | 4 | tickets per wave (at most 8) |
| `attempts` | 2 | passes of a ticket's work inside one run |
| `flow` | `auto` | `dev` or `plain` to force one |
| `approve` | `auto` | `on` adds a Plan card, a Design card when the design run leaves questions, and the merge and discard options of the Finish card; `auto` gets the Finish card only to ask before a push |
| `deadline_min` / `verify_deadline_min` | 20 / 40 | node timeouts of work and check nodes |
| `board` | `auto` | every run is on the board, focused or full; `on` also makes a small goal a run; `off` keeps a run off it: plan, results and report in the chat, no resume |
| `design` | `auto` | the design run before planning, sized by the goal: none when the agent can write the tickets from what it read, explorers and a planner when it cannot, and the panel of three competing designs with a refuter each only when the approach is a real choice that is costly to reverse or the goal asks for a design. `on` forces the panel, `off` none. The run ticket's `Design` bullet says which and why |
| `critic` / `synthesize` | `auto` / `on` | the final review (`auto`: with two or more verified tickets, a hard ticket, or a goal that asks for a review; `on` always, `off` never) / the merged report |
| `models` | `auto` | `auto` takes each stage's model from `agents/workflow-models.md`, and for a provider with no row there runs the explore, baseline and plain work nodes one size below the orchestrator's model, never on its provider's smallest; `own` keeps every node without a pin or a model arg on the orchestrator's model |
| `skip_models` | none | model ids, comma-separated, that `models=auto` never picks; the run adds a picked model that fails, and the run ticket keeps the list for a resume |
| `work_model` / `check_model` | picked by `models` | models of unpinned work and check nodes |
| `plan_model` | `check_model`, else picked by `models` | model of the design, refute and plan nodes, the review's refuters, a hard ticket's work and a stalled batch's repair |
| `budget_usd` | none | stop dispatching once the run's cost reaches it |
| `max_items` | 2 × concurrency, at least 4 | tickets the run may create in all, new items included; the rest go to a Follow-ups ticket |
| `min_free_gb` | 5 | free disk the dev flow needs before it starts or tests |
| `project` | picked by the run | kanban project (id or name) to file the run under; see [Projects](#projects) |
| `resume` | none | an earlier run ticket to adopt |

Every run's nodes carry a phase, in the order the run goes: `baseline`, `explore`, `design`, `refute`, `plan`, `implement` (plain flow: `work`), `verify`, `review`. The ADE's Workflow section lists them per run.

Watch the run on the board, or open the run ticket's waves with `workflow::status {"run_id":"…","details":true}`: phases, labels, the model and usage of every node.

## Steer it

- Add a subtask of the run ticket in To do: it runs next round.
- Comment on a ticket: its next attempt reads the comment.
- Drag a Backlog card to To do to re-queue it. A card in a running wave stays in To do; the ADE's Workflow section shows what is running.
- Type `stop` to abort, `continue` to pick up after an interruption or a report that left work, or start a new session with `args: resume=<run ticket>`.

## Limits

- One `workflow` worker per namespace.
- A node runs as its own session: a session's hard cost cap and its own `options.functions` don't reach the nodes. Every node gets `*` minus the kit's deny list (no board writes, no state writes, no sub-agents, no `workflow::*`), and every node's profile forbids deploying or pointing a command at production or real user data unless its card asks for it. Use `budget_usd` to bound a run.
- A node timeout kills that node only; the run reads it as a failed attempt.
- A page check uses one tab of the browser worker at a time per node, and the worker keeps few tabs open (`max_sessions`, 4 by default): with more page checks than that running at once, a check may lose its tab and start over once. The card names the server's port, so two runs that check the same project at the same time collide; the second check answers `unsure`.
- A run's end reaches the Workflow agent as a `workflow::on-run-completed` notification: a policy that narrows that session's `options.functions` must allow `harness::observe` and `workflow::on-run-completed`.
- The dev flow keeps the branch unless the Finish answer says otherwise. The kit pushes only on that answer, and only the run branch, never the base: when the repository has a remote, the Finish card offers to push the branch and open a ready pull request into the base with `gh`; a branch that already has one gets its title and body updated. A run that stopped `blocked` gets no Finish card until you ask for a push. Only an answer that names the push or the pull request counts; any other reply keeps the branch. The run branch starts from your local base, so the card says when the push would also publish base commits the remote does not have. Without `gh`, or when it is not signed in, the branch is pushed and you get the command to run. A run started by another session has nobody to answer the card and keeps the branch. The kit never watches CI or merges the pull request.
- A Workflow agent started by another session has nobody to ask: the interview takes its recommendations, and a goal that leaves a consequential decision open (scope, compatibility, security, data loss, external cost) ends `blocked` with its questions. State those decisions in the goal.
- A check the baseline could not run for lack of what only you can supply or allow (a download, a credential, a service) is asked once, before the first wave: supply or allow it, or the tickets that need it are delivered `unverified`. With nobody to answer, they are delivered `unverified`.
- The direct path edits your checkout in place, with guarded writes, and commits only when your message asks for it (never a file that already had your uncommitted changes), and pushes nothing. A file that already had uncommitted changes is edited too, and the report names it as mixed with yours: `git diff` is the review, and the report's undo line covers only the files that were clean.
- A direct change is checked by one narrow command the agent picks and names in its report, not by an independent check node. Ask for a workflow run or an independent check in the goal to get one.
- A run with `board=off` cannot be resumed; its branch and worktree are kept.
- A focused run runs no baseline. In a repository whose base is red, a failure whose file, and what that file imports, the change did not touch is reported as `base?` and does not fail the ticket; the report lists it under Needs you.
