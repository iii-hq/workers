# Control tower

Status: proposed, 2026-11-21. Revised twice the same day, after two rounds of
adversarial review (UI/UX and spec). Design canvas:
<https://claude.ai/artifact/BWNQBKBBseVqzNgwXpvz9S> (private until shared).

Base: `fix/deepwork-create-all-items` at `45304ce05`. Three pieces this spec
builds on are not on `main` yet:
- board dispatch (`dd2652220`) and deepwork's `ask=board` (`0eb48c2e0`), on
  `feat/fabrica-autonoma`;
- `harness::on-turn-overdue` (`overdue.rs`), on `feat/watchdog-turn-overdue`.

## Why

Agents build the project; a person steers it. The control tower is the one
screen in the ADE where that person looks and sees the project as it is right
now:

- what the agents are doing;
- how far each release has come;
- which pull requests are open, and why each one waits;
- what changed since they last looked;
- what waits on them.

The last point is **Needs you**, one feature of the tower.

The tower is global and generic. It reads harness sessions, harness asks,
approval-gate holds, kanban cards and releases, and GitHub pull requests.
Nothing in it knows about a kit: a kit, a profile or a plain chat session shows
up there by using those contracts. Counts and states are read live from those
contracts. The prose (the summary, the diary, what each session is doing, why
each PR waits) is written by an agent, the tower writer (§3). It never states a
count or a duration, and every line names what it is about.

Two towers were built by hand in November, and both converged on the same
sections:
- **Assistente R1** (KAN-616): `snapshot.py`, an artifact and a polling loop.
- **billing-v2**: a session writing its own report:
  - five summary tiles;
  - the plan as done, now and next;
  - what needs you, split into now and no rush;
  - what is running, and the PRs with why each one waits;
  - the board by column, and a diary of milestones.

Both covered one session, were refreshed by hand or by polling, and lived
outside the product. The R1 run also showed what costs the most. It was not
the code. Decisions waited for hours, spread over five places that do not know
about each other:

- `harness::ask` cards in a chat nobody had open;
- `⛔ Needs you` comments on the board (`ask=board`);
- Backlog cards labelled `pending`;
- calls held by `approval-gate`;
- prose in the coordinating session.

## Goals and non-goals

Goals:

- One screen gives the portrait of the project: progress by release, the
  sessions at work, the open PRs with their CI, the latest milestones, and what
  waits on the person.
- Every question that blocks work reaches one inbox, whoever asked and however
  the session was started (chat, CLI, board dispatch, cron).
- Only a person answers a question. The answer reaches the session that asked
  at most once, and closes the question everywhere.
- A question that blocks a card is kept on that card, so the release keeps its
  decision log.
- A person can talk a decision through with the session that asked, without
  disturbing it.
- Facts come from contracts. The model writes prose only, with references the
  UI checks.

Non-goals (v1):

- Anything specific to a kit. deepwork and the watchdog adopt the contracts;
  the tower does not read their labels or pills.
- A new worker or datastore. The harness keeps asks in its private `state`
  scopes, kanban keeps the decision log, and the tower writer is a kit (no
  code).
- Parsing answers with a model. Answers are structured; free text stays free
  text.
- Notifications outside the ADE (Telegram, Slack).
- Machine metrics (RAM, `/tmp`, disk). A session that cannot proceed for an
  environmental reason raises a question like any other.
- Project-specific panels, such as billing-v2's production cluster. See Open
  decisions.
- Letting sub-agents ask a person. Depth > 0 still reports `blocked` to its
  parent, and the parent consolidates the questions and asks.
- Stopping an agent that has `shell::exec` from calling any function through
  the `iii` CLI. That gap exists today for `approval::resolve` too, and is not
  this spec's to close.

## What exists today

| Need | Contract | Where | Gap |
|---|---|---|---|
| Ask a person | `harness::ask` (1–4 questions, 2–4 options, no recommended flag). It usually ends the turn, but not when there is steering, a `submit_result` in the same step, or parked calls. The card still takes an answer after the chat moves on. | `harness/src/ask.rs:30,56-63`, `turn_loop.rs:2088,2488-2500`, `harness/ui/src/ask/index.tsx:33-36` | The card lives only in the transcript. No event fires, and nothing lists open asks. |
| Know a session waits | `status: done` + `status_reason: "awaiting answer"` when the turn ended on the ask | `harness/src/session_status.rs:30,52` | `harness::on-turn-completed` carries no reason (`turn_loop.rs:3049-3056`), and `harness::status` does not report it. |
| Close a card | `harness::ask::settle` (replaces, does not refuse) and `settlements` (newest 32 per session) | `harness/src/functions/ask_settle.rs:60`, `ask_settlement.rs:47-56` | Per session. No record of who answered. Two answers both land. |
| Deliver once | `harness::send {idempotency_key}`: a repeated key returns the original turn and appends nothing | `harness/src/functions/send.rs:153-156` | — |
| Private state | The harness claims private scopes at boot. `harness::state::*` is in `agent_call_forbidden`, which applies whatever the policy says; console and operator calls bypass it. | `harness/src/state.rs:795-806`, `functions/observe.rs:398-418` | Public scopes are readable by agents (`state::get` and `state::list` are default allows, `iii-permissions.yaml:414-415`). |
| Who may call what | `FunctionPolicy {allow, deny, expose}`: globs over function ids. A child gets the subset of its parent's policy. An explicit `options.functions` on a send replaces the policy. | `harness/src/types/turn.rs:58`, `policy.rs:188-212`, `functions/send.rs:1480-1493` | A policy cannot confine `state::set` to one scope. |
| Held calls | `approval::list-pending`, `resolve {decision}`, `approve-always`, `on-pending-created` / `on-pending-resolved`. `list-pending` is always denied to agents. `session::list`, `session::messages-tail` and `harness::ask::open`-like reads are held in manual mode by the default rules. | `approval-gate/src/types.rs:171-442`, `decision.rs:19-21`, `permissions/default_rules.rs:10-12` | `assistant_excerpt` is always omitted (`types.rs:196`): a hold carries the function, a redacted `arguments_excerpt` and the session title, not a "why". |
| Card a question blocks | `Comment {id, ticket_id, parent_id, body, author, created_at}`: append-only; only `comment.created` | `kanban/src/store.rs:30,234`, `events.rs:196` | A question is a text convention that the model parses. |
| Questions without a chat | deepwork `ask=board`: a `⛔ Needs you` comment, the pill, a wait on `kanban::on-comment`, a model that matches the reply | `kits/deepwork/agents/deepwork.md` at `0eb48c2e0` | It lives in a prompt, for one kit. Its comment format is shared with dispatch's park comments. |
| Board dispatch | It starts the assignee's session for a To do card (`metadata.kanban_ticket`), sends `ask=board` in every first message, and on a terminal turn **parks the card** in Backlog with `unfinished` and the `needs you` pill, at boot too | `kanban/src/dispatch.rs:21-22,365,398,640-646,~728` at `dd2652220` | A turn that ends on a question looks like a stop. |
| Session ↔ card | `metadata.kanban_ticket`, set by dispatch. `session::set-meta` replaces the whole metadata and is denied to agents. | `session-manager/src/functions/set_meta.rs:11-18`, `iii-permissions.yaml:126` | No safe way for an agent to link its own session. |
| Sub-agents in lists | Children inherit `kind` and carry `metadata.spawned_by: "agent"` | `harness/src/subagent.rs:910-937` | `session::list` cannot ask for roots only. |
| Releases and progress | `kanban::release::list`: `status`, `target_at`, `total`, `done`, `counts`, `projects[]`. List rows without `compact` carry `labels`, `pills`, `updated_at`, `release_id`. | `kanban/src/plan.rs:64`, `board.rs:83-99` | No count of what needs you. |
| What changed | `kanban::activity::list {ticket_id, types?, since?, limit?}` | `kanban/src/functions.rs:304` | One ticket at a time. |
| Pull requests | `github::pr::list {repo, state}`; `view` adds `body`, `mergeable`, `reviewDecision`; `checks` per PR | `github/src/functions/pr.rs:11-17,40,102` | It needs `owner/name`, which a kanban project does not carry (`store.rs:118`). deepwork opens PRs with `gh` through `shell::exec`. |
| Side chat | `session::fork {session_id, entry_id, title}` tagged `side_chat_of`, reused per conversation; it copies `kind` and `metadata` | `ade/web/src/components/chat/ChatAside.tsx:255-298`, `session-manager/src/service.rs:1445` | It gets the console's policy; there is no read-only mode. |
| Live data in pages | `useWorkerLive` refetches everything on every event of its triggers | `packages/console-ui/hooks.mjs:155-215` | Chatty events mean constant refetches. |

## Design

### 1. The screen

A built-in ADE screen, `tower`, next to `workers` and `traces`
(`ade/web/src/lib/workspace-tabs.ts`). It is built-in because it reads several
workers, and an injected page may only call its own worker. It covers the whole
project: one harness, one board.

- **Header on every view:**
  - "Control tower";
  - the **scope**: All projects, or one project. The scope governs every view.
    With All projects, the Board card shows the whole board;
  - the summary's freshness, always in the same words ("Summary 4 min ago",
    with a warning past 30 min);
  - Refresh.
- **Views, the same set everywhere:** Overview (§2), Needs you, Assumptions and
  Decided (§6), Runs (§7). Full Pull requests and Activity views come later.
- **Phone, under 640 px:** a segmented control (Overview, Needs you, Runs).
  Needs you links to Assumptions and Decided at its top.
- **Live data.** Each source keeps its own fetcher, debounced to 2 s:
  - **Needs you:** `harness::on-ask`, `harness::on-ask-settled`,
    `approval::on-pending-created`, `approval::on-pending-resolved`;
  - **Board:** `kanban::on-change`;
  - **Sessions:** `harness::on-turn-started` and `harness::on-turn-completed`,
    filtered to depth 0;
  - **Summary:** `harness::on-turn-completed`, filtered to the writer
    session (§3);
  - **PRs:** a 5 min timer while the screen is open, and on focus.

  `session::on-status-changed` and `state::on-changed` are not observed.

### 2. Overview: the portrait

Canvas artboard `Overview`. Two columns. The left one comes first on a phone.

| Section | Facts, read live | Prose, from the summary |
|---|---|---|
| Summary | — | `brief.headline`, `brief.text`, references as links |
| Tiles | Needs you (count, and how many are Now); Working (root sessions with a turn running in their tree); Waiting (on an answer, on a held call, or stalled); Open PRs (how many have CI failing); Active releases | — |
| Needs you (left) | the top 3 Now items in §6.1 order, each with Open and Ask about this; a "No rush" button with its count; a link to all | — |
| Diary (left) | — | today's newest `diary` lines, each with its time and reference |
| Releases (right) | `kanban::release::list`: the newest 2 released, then active, then planned by `target_at`; each with done/total, `open_questions` and what blocks it | — |
| In progress (right) | the runs of §7: title, state, open questions, age | `now[session:<id>]` |
| Pull requests (right) | §4: `owner/repo#n`, title, CI state, linked card, age | `pr_waits[<owner/repo#n>]` |
| Board (right) | the scope's columns: done/total; In progress, In review and To do rows with their pills ("+n more" opens the board page); Backlog and Done folded | — |

- **One primary action per view.** The Overview has none: its Needs you items
  open the item in Needs you.
- **Status words stay ink.** Color goes only on the dot.
- **Every row links:** a run to Runs, a card to its ticket, a session to its
  chat, a PR to GitHub, a diary line to its reference.
- **Prose is marked:** "agent · 4m" on an agent-written line; "from the
  transcript" or "from GitHub" on a fallback line.
- **Without a fresh summary** (stale over 30 min, absent, or the kit not
  installed):
  - the Summary and Diary become a one-line state, with Refresh;
  - In progress falls back to the session's last assistant message, cut to one
    line;
  - Pull requests fall back to a deterministic reason: draft, CI failing,
    review requested, or merge conflict.
- **States** (canvas `Overview and Runs states`):
  - loading;
  - GitHub unreachable: PRs not listed, the rest unchanged;
  - a project without repositories;
  - summary stale;
  - summary absent;
  - an empty project;
  - Runs loading;
  - Runs failed.

### 3. The summary: the tower writer

The tower writer is a kit, `tower`, with no code. It runs only when someone is
looking.

- **Session.** One automation session per scope: `tower-all`, or
  `tower-<project id>` for a single project. It has `metadata.origin: tower`,
  is created on first use, and is left out of Runs.
- **Trigger.** The ADE sends it a message when the tower screen is open:
  - on open, when its last result is more than 10 min old;
  - every 10 min while the screen stays open;
  - on Refresh.

  Each send uses `idempotency_key: tower:<scope>:<10-min slot>`, so two open
  tabs send once.
  - `ponytail:` the summary is only as fresh as the last time someone looked.
    Add a cron trigger if it must be ready before anyone opens the screen.
- **What it may call.** Each send sets `options.functions` to reads only, with
  no write function anywhere:
  - `kanban::ticket::list`, `kanban::activity::list`,
    `kanban::release::list`;
  - `session::list`, `session::messages-tail`, `harness::status`,
    `harness::ask::list`;
  - `github::pr::list`, `github::pr::checks`.

  Its inputs are text written by other agents and people: comments, PR titles,
  assistant messages. Text that tries to make it act has nothing to act with.
  It does not read approval holds: `approval::list-pending` is denied to
  agents, and the screen already shows the holds.
- **Approval rules.** The kit ships `iii-permissions.yaml` allow rules for
  those reads in sessions `tower-*`. Without them, the approval-gate's default
  rules would hold its reads, and each refresh would land in Needs you.
- **What it reads:**
  - the message: the previous result's `checked_at`;
  - its own transcript, which holds its previous results, so the diary carries
    over;
  - what changed since then: cards (rows without `compact`, `updated_since`),
    board-wide activity (§5), root sessions whose turn completed (the last
    assistant message, cut short), open PRs and their checks, open questions.

  It never reads whole transcripts. Compaction keeps its session bounded.
- **Result.** The send sets `options.output` (a JSON contract). The result:

  ```jsonc
  {
    "checked_at": "2026-11-21T18:44:00Z",
    "unchanged": false,      // true: nothing new; the screen keeps the last prose, freshness = checked_at
    "brief": { "headline": "…", "text": "…", "refs": ["KAN-616", "KAN-597", "iii-hq/workers#1834"] },
    "diary": [ { "at": "…", "text": "KAN-635 verified after the second branch review.", "refs": ["KAN-635"] } ],
    "now": { "session:<id>": { "text": "Running the verify checks of the round." } },
    "pr_waits": { "iii-hq/iii-cloud#451": { "text": "Draft until the run's last cards are verified." } }
  }
  ```

  The screen reads it from `harness::on-turn-completed` for the writer session,
  and on reload from `harness::status` `result`. A result that fails the
  contract keeps the previous one on screen.
- **Prose rules.**
  - The writer never states a count or a duration. Counts sit beside the prose,
    read from contracts.
  - Names that contain digits (R1, round 23, billing-v2) are fine.
  - References take one of three forms: a card key, `session:<id>`, or
    `owner/repo#n`.
  - The screen drops a reference that does not resolve, and drops a `now` or
    `pr_waits` line whose subject is no longer live.
  - Prose renders as plain text, never markup.
- **Language:** a kit arg, English by default (the board's language).

### 4. Pull requests

- **Repositories.** Kanban projects gain an optional `repos: ["owner/name"]`,
  edited on the Projects page. A project without it falls back to the `origin`
  remotes of the roots its sessions worked in. The ADE's Context tab already
  reads remotes this way, with `shell::exec` running `git`.
- **List:** `github::pr::list {repo, state: "open"}` per repository, plus PRs
  merged in the last 24 h for the diary. Then, per open PR,
  `github::pr::checks` (passing, failing, running) and `github::pr::view`
  (`reviewDecision`, `mergeable`). Two calls per open PR every 5 min.
  `ponytail:` fine for tens of PRs; batch through `github::exec` with `gh pr
  list --json statusCheckRollup` if a project has more.
- **Card link:** a card key (kanban `id_prefix`) in the PR's title or head
  branch.
- **Session link:** through the card (card → its run, §7), never from
  transcripts.
- **Live:** no webhooks in v1. `pr::on-event` only covers PRs under a watch
  lease.

### 5. What changed: board-wide activity

`kanban::activity::list` gains a board-wide form. Without `ticket_id`, it
returns every ticket's activity, newest first, narrowed by `since`, `types`
and `limit`.

### 6. Needs you

#### 6.1 Items and urgency

Three kinds of item, labelled the same in every view:
- **Question · <class>:** an open `harness::ask` (§6.2).
- **Held call:** an approval-gate pending record.
- **Stalled run:** computed by the tower (§7), not asked by anyone.

Urgency, one rule for every view:
- **Now:**
  - held calls and stalled runs;
  - questions of class `run_health`, `security`, `publish`, `external_cost` or
    `data`;
  - any question whose `about.blocks` is not empty.
- **No rush:** `scope` and `routine` questions that block nothing.

Order within a group: the number of cards blocked, largest first, then age,
oldest first. One sort function feeds every view.

#### 6.2 Harness: asks that stay open until a person answers

The question card gains optional fields. The harness passes them through:

```jsonc
{
  "questions": [ {
      "header": "Builds", "question": "…", "multi_select": false,
      "options": [ { "label": "Isolate builds", "description": "…", "recommended": true },   // new: at most one
                   { "label": "Allow-list the network", "description": "…" } ] } ],
  "about": {
    "ticket": "KAN-597",        // the card it blocks (else the session's metadata.kanban_ticket)
    "blocks": ["KAN-598"],      // the cards that wait on it
    "class": "security",        // security | publish | external_cost | data | scope | run_health | routine (default scope)
    "why": "…"                  // one paragraph, shown above the options
  },
  "expires_in_min": 30,         // no value waits forever, like a held call
  "on_expiry": { "answer": "Retry only the unfinished checks" }   // or "block" (the default)
}
```

**Lifecycle.** The harness writes an entry when it **accepts** the ask. The
entry stays open while the session goes on with other work. A person may
answer later; the answer arrives as a message, as the chat card already allows
today. States:
- `open`;
- `answering`: reserved, being delivered;
- `answered`, `dismissed`, `withdrawn`: closed;
- `expired`: the expiry text was delivered; the question still takes an
  answer.

Closed entries are kept, with their question, answer and `by`, until the
session is deleted (`session::on-deleted` purges them).

**Store.** A private harness scope, `harness_ask`, claimed with the others at
boot (`harness/src/state.rs:801`). Keys are `session_id/question_id`, because a
fork copies call ids. All changes go through `harness::state::compare-and-set`,
which `agent_call_forbidden` keeps from agents. Public scopes are readable by
agents, so the store cannot live in one.

**Answering.** Only people answer, through
`harness::ask::answer {session_id, question_id, answers | dismissed, by}`:

```jsonc
{ "answers": [ { "header": "Builds", "choice": "Isolate builds" },     // single select
               { "header": "Channels", "choices": ["Console"] },        // multi select
               { "header": "Region", "other": "eu-west-2" },            // free text
               { "header": "Budget", "skipped": true } ],
  "by": { "channel": "tower", "user": "<the person>" } }                // channel: chat | tower | kanban | expiry
```

1. **Reserve.** Compare-and-set from `open` or `expired` to `answering`. A
   second caller gets `already answered by <channel>`.
2. **Deliver.** `harness::send`, with the text the chat card sends today and
   `idempotency_key: ask:<session_id>/<question_id>`. The send starts a turn,
   or merges or queues into a running one.
3. **Close and emit.** The entry is closed, and `harness::on-ask-settled` is
   emitted, carrying the whole entry: question, about, answer, by and outcome.

A crash after the reservation leaves `answering`. At boot the harness repeats
step 2 with the same key, which delivers at most once, and then closes the
entry.

**Who may answer.**
- `harness::ask::answer` and `kanban::question::answer` join
  `agent_call_forbidden` (`functions/observe.rs:402`). No policy can give
  them to an agent, while console and operator calls pass. The chat card,
  Needs you and the kanban ticket screen are console calls.
- **Let the agent decide** sends `dismissed`. It is only offered for `scope`,
  `routine` and `run_health` questions.

**Withdrawing.** `harness::ask::withdraw {question_id}` is callable by an
agent, and only for its own session: the harness stamps the caller's session,
like `with_caller_session_id`. An agent calls it when the question no longer
needs an answer.

**Expiry.**
- One deadline per open entry, armed on accept and by the boot read.
  `ponytail:` in-process, like `overdue.rs`; a deadline that passes while the
  harness is down fires at boot.
- The deadline fires only on an entry that is still `open`, and it first moves
  it with compare-and-set (so a second harness process does not repeat it).
- **With `on_expiry.answer`:** the harness answers with that option, `by:
  {channel: expiry}`, adding ` (no answer within <n> min; default applied)`.
  The option must be a label of the only question, which must be
  single-select, and the class must be `run_health` or `routine`. Anything
  else is refused at ask time.
- **With `block`:** the entry becomes `expired`, and the harness delivers `No
  answer within <n> min to: <header>. Do not decide it yourself. If you
  cannot go on without it, finish blocked; the answer will arrive as a
  message.` (idempotency key `ask-expiry:<session_id>/<question_id>`).

**Signals for other workers.** `harness::status` and
`harness::on-turn-completed` gain `open_asks` (the count of the session's open
or expired entries). Board dispatch reads it (§6.3).

**Events and listing.** `harness::on-ask` (on accept) and
`harness::on-ask-settled`. `harness::ask::list {session_id?, state?, limit?,
cursor?}` lists entries, the closed ones included.

**The chat card** switches to `harness::ask::answer`. An ask with no entry
(older than the deploy) keeps today's path: the answer goes out as a composed
message.

#### 6.3 Kanban: the question lives on the card it blocks

Comments stay append-only.

- **Question comment.** `Comment.question` (optional): `{session_id,
  question_id, questions, about, asked_at}`. Its `body` keeps a plain-text
  rendering for agents and older UIs.
- **Answer comment.** `Comment.answer` (optional): `{answer_text, by,
  outcome}`, written as a reply to the question comment. `outcome` is one of
  `answered`, `dismissed`, `withdrawn`, `expired_default` or `not_delivered`
  (the session was gone when the person answered; the answer is kept here).
- **Board version.** Both fields need `BOARD_VERSION` 3, so a version 2 worker
  refuses the file instead of dropping them (`kanban/src/store.rs:234`).
- **Mirror.** The kanban worker observes `harness::on-ask` and
  `harness::on-ask-settled`:
  - It resolves the card from `about.ticket`, else from the session's
    `metadata.kanban_ticket`.
  - The settled event carries the whole entry, so a settle that arrives first
    writes both comments, in order. Writes are idempotent on
    `session_id/question_id`.
  - At boot it reconciles against `harness::ask::list`.
  - The harness owns the state; the board shows it.
  - A question with no card is not mirrored, and is listed under "Not on the
    board".
- **`kanban::question::answer {ticket_id, comment_id, answers | dismissed,
  user}`** forwards to `harness::ask::answer`. The ticket screen's injected UI
  can only call its own worker. It is in `agent_call_forbidden` too.
- **`kanban::question::list {state?, project?, release?, limit?, cursor?}`**:
  question comments with their state, for the Decided log.
- **`kanban::question::record`** writes a question with its answer at once,
  with `recorded_by: <profile>`. An agent uses it for a material choice it made
  alone, and the tower lists it under Assumptions.
- **Derived mark.** Ticket views, `kanban::release::list` and
  `kanban::project::list` gain `open_questions`. The board shows `needs you`
  while a card's count is above zero, and the kanban worker writes no pill for
  it.
- **Dispatch fix.**
  - A dispatched session whose terminal turn reports `open_asks > 0`, on the
    event or at boot through `harness::status`, is not parked: its card stays
    In progress, with the derived mark.
  - A real stop still parks the card, with the pill `unfinished` instead of
    `needs you`.
  - Dispatch stops sending `ask=board`.

The ticket screen renders the pair as one card (canvas `Ticket`):
- the options, with the recommended one preselected only when there is one;
- "Or answer in your own words": typing clears the option, and clearing the
  text brings back only a recommended option;
- Answer, and Let the agent decide where §6.2 allows it.

Once answered, the card collapses to what was answered, by whom, and the
outcome.

#### 6.4 The Needs you, Assumptions and Decided views

Canvas artboards `Needs you`, `Side chat`, `Phone`, `Phone answer`,
`Phone held call`, `Edge states`.

- **Rail:** the releases of the scope, plus "Not on the board", each with its
  count of open items. A Sources line counts questions, held calls and stalled
  runs; it is text, not a control.
- **List:** Now and No rush, in the §6.1 order. Each row carries its kind
  label.
- **Detail pane, for a question:**
  - the kind label and the question;
  - who asked: profile, session, card and age;
  - `about.why`;
  - what waits on it;
  - the options, plus free text;
  - the expiry policy in one line;
  - the trail;
  - **Ask about this**, with the asker's context size (§6.5);
  - **Answer**, the only primary, disabled until there is an answer;
  - **Let the agent decide**, where allowed.

  When the session has moved on since it asked, a neutral line says: "The
  session moved on since it asked; your answer arrives as a message."
- **Detail pane, for a held call:**
  - shown: the function, the redacted `arguments_excerpt`, the session title
    and its card;
  - not shown: no "why" in prose, no preselection, no "Recommended", no free
    text, no Let the agent decide;
  - three explicit buttons: Deny, Allow once (`approval::resolve`), and Always
    allow in this session (`approval::approve-always`).
- **Detail pane, for a stalled run:** the stall facts (§7), and two actions.
  Resume sends "continue" to the root session. Leave it hides the item until
  the run changes.
- **Send states:**
  - Sending: the options give way to a summary of what was sent;
  - Delivered;
  - Failed, with Retry;
  - Already answered by `<channel>`: the item closes in every tab;
  - Withdrawn by the asker.
- **Assumptions:** the recorded questions. Keep hides one. "Reopen as a
  question" posts a person comment on its card, which the agent reads like any
  comment that steers its work.
- **Decided:** `kanban::question::list` for closed questions: question,
  answer, by, when and outcome. A `not_delivered` row offers Open card. Held
  calls are not listed, because the approval-gate keeps no history.
- **On reload,** the view refetches `harness::ask::list`,
  `approval::list-pending` and `kanban::question::list`, and merges them by
  `session_id/question_id`. The harness state wins.
- **Edge states:**
  - Nothing needs you;
  - loading;
  - a source down;
  - failed to send;
  - not delivered;
  - expired;
  - already answered;
  - withdrawn.
- **Phone:** a scrolling list with 60 px rows. A question opens a bottom sheet
  (canvas `Phone answer`), and a held call opens its own sheet with the three
  buttons (canvas `Phone held call`). Both sheets scroll, with their actions
  pinned at the bottom, and the question sheet has Ask about this.

#### 6.5 Side chat: think a decision through before answering

Canvas artboard `Side chat`. "Ask about this" opens the ADE's side chat beside
the decision, on a copy of the session that asked. The copy knows why it asked,
what it tried and what each option costs, and the original session is left
untouched.

- **One copy per question,** keyed `session_id/question_id`, never the
  conversation's ordinary side chat (`ensureFork` reuses that one).
- **Fork point.** The copy is forked at the entry before the ask call, so it
  does not end on an open card. Otherwise its first message would be read as
  the answer. That first message quotes the assistant text of the step that
  asked, the question, its options and `about.why`, and says this is a
  conversation about a pending question.
- **Metadata.** The copy keeps `side_chat_of` and `side_chat_question`. It
  drops `kanban_ticket`, so it never shows up in Runs (Runs also skips
  `side_chat_of`).
- **Read only, enforced by the harness.** For a session whose metadata has
  `side_chat_question`, the harness intersects every send's policy with the
  harness configuration `side_chat_functions`. The check goes in
  `functions/send.rs`, before `resolve_send_agent`. The default list:
  `session::get`, `session::messages-tail`, `harness::status`,
  `kanban::ticket::get`, `kanban::ticket::list`, `kanban::question::list`,
  `coder::read-file` and `memory::recall`.
  - The client cannot widen it.
  - The ADE sets the copy's approval-gate always-allow for the same reads
    (`approval::add_always_allow`, a console call), so they are not held and
    do not land in Needs you.
  - `shell::exec` is out, so the copy cannot run git.
- **The person answers, in the pane.** The side chat may propose an option or
  a text, and "Use this answer" fills the pane, free text included. Answer
  still calls `harness::ask::answer` on the original session. The copy cannot
  answer: the function is in `agent_call_forbidden`.
- **Held calls.** A held call has no question, so the copy forks the held
  session at the entry before the held call. That `session::fork` of a session
  parked on a hold is not verified yet (Validation 4).
- **Session gone:** the side chat starts a fresh session seeded with the
  question, the card and the trail.
- **Cost.** The copy's first turn rereads the asker's whole context, and it
  runs without the asker's kit system prompt, so the prefix cache does not
  help. The button shows the context size before opening.
- **Lifetime.** The copy is deleted when the question closes, unless the
  person presses Keep.
- **Phone:** full screen, as the ADE's side chat already is there.

### 7. Runs: every session doing work

Canvas artboard `Runs`. A run is a root session that is not a side chat and
not a writer, of kind `user` or `automation`, that either:
- has a turn running in its tree, open questions, a held call, or a wake
  armed; or
- is linked to a card and ended in the last 24 h.

A chat that simply ended is not a run.

- **Session list.** `session::list` gains `roots: true` (sessions without
  `spawned_by`). Without it, the 200-row page fills with sub-agents.
- **Linking to a card:**
  - board dispatch sets `metadata.kanban_ticket`;
  - any other session calls `harness::link-card {ticket}`, which is new. It is
    callable by agents, and the harness writes `kanban_ticket` into the
    **caller's own** root session (stamped like `with_caller_session_id`,
    merged into the existing metadata). `session::set-meta` stays denied to
    agents: it replaces the whole object and does not check whose session it
    is.
- **Not linked:** the session is listed under "Not on the board".

| Shown | From |
|---|---|
| Title, profile, how it started | `session::get`; "board dispatch" when the session id starts with `dispatch-` |
| Progress per release | the card's subtasks, `kanban::ticket::list {parent_id: <card>}` without `compact`, grouped by `release_id`, with a legend (done, in progress) and an `aria-label` on each bar |
| Pills | the card's pills, as the agent set them |
| State and age | `session::get` and `harness::status` (`open_asks`, `expects_wake`, `armed_wakes`, `children`) |
| What it is doing | the summary's `now[session:<id>]`, else the last assistant message, cut to one line |
| Last progress | the newest `updated_at` among the card and its subtasks, else the session's |
| Needs you | the open items of the session, of its children, and of its card |
| PRs | the PRs linked to its card (§4) |
| Actions | Open session (its chat); Open card (only with a card) |
| Usage | `harness::metrics` for the session's subtree |

A run is **stalled** when every one of these holds:
- its root is `done`, with no running children and no open questions;
- it has wakes armed (`expects_wake`), but none of them has a future deadline
  (`armed_wakes[].expires_at`);
- nothing has changed for `stall_after_min` (ADE configuration
  `tower.stall_after_min`, default 30).

A wait with a deadline in the future is not a stall. A stalled run is an item
of Needs you (§6.4).

### 8. Agents and kits

These rules are generic. They go in the harness README's `harness::ask`
section and in the kanban skill, not in a kit.

- **Ask from the root.** A session at depth 0 asks with `harness::ask`, with
  `about` (and `about.why`) when its work is on a card, and marks the option it
  recommends. A sub-agent reports `blocked` with its question to its parent,
  and the parent asks.
- **Keep going if you can.** A question stays open while the session does other
  work, and its answer arrives as a message. Withdraw a question
  (`harness::ask::withdraw`) once it no longer needs an answer.
- **Link the work.** A session that works on a card calls `harness::link-card`.
- **Name the card in the PR.** A PR for a card names its key in the title or
  the branch.
- **Wait with a question, not a block.** A session that cannot go on for a
  run-health reason (a check that keeps timing out, an outside service down)
  asks a `run_health` question with a default, instead of ending `blocked`.
- **Record what you decided alone.** A material choice made without asking
  goes in `kanban::question::record`.

First adopters:

- **deepwork:**
  - drops `ask=board`; it already treats a user message as the answer to its
    open card (`deepwork.md`, "User message mid-run");
  - turns `ask_wait_min` into `expires_in_min` with `on_expiry: "block"`;
  - stops writing `needs you` pills;
  - calls `harness::link-card` at Step A;
  - names R in its PR titles;
  - turns a verify that times out twice into a `run_health` question;
  - records the material assumptions of `approve=auto`.
- **The watchdog kit:** it does not ask. Its escalation to a person is the
  tower's stalled-run item, which settles the watchdog spec's open escalation
  decision.
- **The tower kit** (§3).

## Phases

1. **A read-only tower.** No new contracts. The tower answers only what today's
   contracts answer safely.
   - **Overview:**
     - the tiles, Releases, Board and In progress;
     - Pull requests, with repositories from the session roots' remotes;
     - no Summary or Diary, and the fallbacks of §2 for the prose.
   - **Needs you:**
     - **Held calls** are answered here, with `approval::resolve` and
       `approve-always`. They are console calls, and the approval-gate already
       refuses a second resolve of a record.
     - **Asks** are listed: sessions with `status_reason: "awaiting answer"`,
       filtered in the client, with the card read from
       `session::messages-tail`. Open opens the session's chat, where the
       existing card answers. Without the reservation of §6.2, the tower does
       not send answers itself.
     - **Stalled runs** are computed, with Resume.
     - **Cards with a `needs you` pill** are not listed as questions. Today
       that pill also marks dispatch's parked cards and deepwork's blocks.
       They show as pills in Board and Runs.
   - **Runs:** as in §7, with roots filtered in the client and only
     dispatch-linked sessions on a card. `ponytail:` one 200-row page.
   - No side chat, Decided, Assumptions or Summary.
2. **Contracts.**
   - Harness:
     - §6.2: the private store, `answer`, `withdraw`, `list`, the events, the
       expiry, `open_asks`, and `recommended` and `about` on the card;
     - `harness::link-card`;
     - the side chat ceiling;
     - the two answer functions in `agent_call_forbidden`.
   - Kanban:
     - §6.3: question and answer comments, `BOARD_VERSION` 3, the mirror,
       `question::*`, `open_questions` and the dispatch fix;
     - `Project.repos` (§4);
     - board-wide activity (§5).
   - Session-manager: `list roots`.
   - ADE:
     - Needs you answers asks itself;
     - the side chat, with its approval allows;
     - Decided and Assumptions;
     - the chat card switches to `answer`;
     - the writer trigger.
   - The `tower` kit with its approval rules (§3).
3. **Adoption** (§8): the generic rules in the docs, then deepwork and the
   watchdog.

## Validation

1. **Phase 1, on the dev stack.** Set up five waiting items:
   - a deepwork run with `approve=on`, started from the board;
   - a held `git push`;
   - a chat session left on an ask card, with no card;
   - a board-dispatched `backend-engineer` session that opens a PR naming its
     card;
   - a root session made to stall (a wake with no deadline that never fires).

   Then check that:
   - **Needs you:**
     - all five show within 2 s, and again after a reload, in the §6.1 order;
     - the held call can only be denied or allowed, with nothing preselected;
     - two quick Allow clicks resolve once;
     - Resume starts a turn on the stalled run;
     - a wait whose deadline is in the future is not marked stalled;
     - a parked dispatch card is not listed as a question.
   - **Overview:**
     - each tile equals a count taken by hand from its source;
     - the PR shows its repository and CI state, and links to its card.
   - **Runs:** the roots are listed, and no sub-agent, side chat, writer or
     `e2e` session is.
2. **Phase 2, harness integration scenario `ask-open-answer`.** It checks
   that:
   - an accepted ask writes an entry, emits `on-ask` and is listed, even when
     the turn goes on (steering in the same step);
   - two concurrent `answer` calls deliver exactly one message, and the second
     gets `already answered`;
   - an answer after the session moved on is delivered as a message;
   - an agent calling `answer`, under any policy, is denied;
   - `withdraw` closes only the caller's own question;
   - a `block` expiry delivers its text once, even with a second harness
     process, and a late answer is then delivered;
   - `on_expiry.answer` on a `security` question is refused at ask time;
   - killing the process after the reservation ends, after boot, with exactly
     one message;
   - deleting the session purges its entries;
   - `open_asks` appears on `harness::status` and on `on-turn-completed`;
   - an agent's `state::get` cannot read `harness_ask`.
3. **Phase 2, kanban.** Unit tests check that:
   - the mirror resolves the card from `about.ticket`, then from
     `metadata.kanban_ticket`, and skips a question with neither;
   - a settle that arrives first writes both comments, in order;
   - the boot reconciliation fixes a missed event;
   - `open_questions` follows ask and settle;
   - a dispatched terminal turn with `open_asks > 0` leaves the card In
     progress, at boot too;
   - a real stop parks the card with `unfinished`;
   - a version 3 board is refused by a version 2 worker;
   - board-wide `activity::list` works.
4. **Phase 2, side chat.** On the deepwork orchestrator's question, check
   that:
   - the copy ends at the entry before the ask;
   - a send with `options.functions: {allow: ["*"]}` is still limited to
     `side_chat_functions`;
   - `kanban::ticket::update`, `harness::spawn` and `harness::ask::answer` are
     refused in the copy;
   - the copy's reads are not held by the approval-gate;
   - the original transcript is unchanged;
   - the copy is absent from Runs, and is deleted when the question closes
     unless kept;
   - the same check passes on a session forked while parked on a held call.
5. **Phase 2, the tower kit,** for a day on a project with live runs, with the
   screen open. Check that:
   - each refresh's policy holds only reads, and no read is held by the
     approval-gate;
   - two open tabs trigger one refresh per slot;
   - every reference resolves;
   - a day of results, read by hand, states no counts or durations;
   - a PR title carrying an instruction changes nothing but prose;
   - an unchanged refresh returns `unchanged: true`, and freshness follows
     `checked_at`;
   - the day's cost, read from `harness::metrics`, is written down.
6. **Phase 2, end to end (harness-e2e).** A board-dispatched session:
   - asks with `about.ticket`, keeps working, and its card stays In progress;
   - gets the answer through `kanban::question::answer`, delivered as a
     message;
   - finishes;
   - leaves the question and its answer comment on the card, with
     `by.channel: kanban`.

   The same answer then sent from the chat card gets `already answered`.
7. **UI.** Every view at phone, narrow split and wide widths, in both themes,
   compared with the canvas through the `design-diff` skill, with the manifest
   free of warnings.

## Open decisions

- **The chat card for a question that has a card:** undecided. With the
  reservation of §6.2, showing it in both places is safe: the first answer
  wins, and the other gets `already answered`.
- **Project-specific cards** (billing-v2's production cluster, environments,
  deploys). Options:
  - a new ADE host slot that lets a worker add a card to the Overview;
  - left out until a second project needs one.
- **Repositories:** `Project.repos` set by hand, plus the fallback (the
  proposal), or the fallback alone.
- **Summary freshness:** refreshed only while someone looks (the proposal), or
  a cron trigger so it is ready on open.
