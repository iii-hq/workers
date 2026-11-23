# Control plane worker

Status: proposed, 2026-11-22. Revision 5, after four adversarial reviews.

**2026-11-23.** Held calls and approval-gate are out of the control plane for
now because the operator does not use approval-gate.

This spec extends `README.md`, the control plane spec, in PR #1855.

It moves the control plane out of the ADE shell into a `control-plane` worker, which owns:
- the screen;
- a read model;
- the schedule of the summary writer.

The page follows the design canvas (§7). An earlier "CEO view" order of the
Overview was withdrawn on 2026-11-23, at the person's request.

The project controller is a separate spec, `controller.md`, which builds on
this one.

The contracts of `README.md` §6.2 (harness asks), §6.3 (the kanban mirror)
and §7 (roots) are unchanged.

## Why

- **A product screen belongs to a worker,** and the ADE stays a generic
  host.
  - kanban, eval and state each own their screen.
  - The prototype was built into the ADE shell (about 9,900 lines with tests).
  - That was a convention, not a technical limit: `host.iii.call` accepts
    any function id (`ade/web/src/lib/console-api.ts:438-447`), and the
    onboarding page already calls `state::get` directly.
- **Every open tab aggregates on its own.**
  - Each tab reads eight sources, and GitHub every 5 min, two calls per
    open PR.
  - N tabs mean N times the reads.
- **The summary runs only while a tab is open.**

## Decisions

These are the defaults, open to review.

- **D1. New commits on top of #1855.** The branch history is not rewritten.
- **D2. The side chat stays in the ADE,** behind a host capability (§5).
  The fork, its metadata and the cleanup are chat infrastructure.
- **D3. The worker schedules the summary.**
- **D4. A TypeScript worker:**
  - `language: javascript`, `iii-sdk` 0.26 for Node;
  - bundled to `dist/`, like `claude-code/` and `cursor/`;
  - with the `ui/` layout of `onboarding/`.

  `model.ts` and `summary.ts` are pure rules. They move as they are and are
  shared by the worker and its page.

  `data.ts` and `use-summary.ts` are React hooks over `window`. They are
  rewritten for the server (§2, §4).
- **D5. Actions stay as console calls the page makes,** as they are today.
  The worker adds no proxy that acts for a person, so no new path leads an
  agent to `harness::ask::answer`.
- **D6. The writer profile ships in a minimal `control-plane` kit.**
  - The harness resolves profiles only through `directory::agents::get`
    (`harness/src/agents.rs:37,126-150`), and iii-directory reads the agents
    folder and installed kits, not a worker's bundle.
  - `kits/control-plane` keeps `iii.kit.yaml`, its README and
    `agents/control-plane-writer.md`.
  - Its `workers` list drops `github`, which stays optional (§1).
  - The result schema and the contract test move to `control-plane/`.

## What moves, what stays

| Item | Stays in the ADE | Moves to `control-plane/` | Rewritten |
|---|---|---|---|
| `index.tsx`, `NeedsYou.tsx`, `answer-state.ts`, `control-plane.css`, `fixtures.ts`, stories and tests | | `control-plane/ui/src/` | |
| `selectionFromSideChat`, `DecisionNeed` (now in `NeedsYou.tsx`, `side-chat.ts`) | | `control-plane/ui/src/` | |
| `model.ts`, `summary.ts`, their tests | | `control-plane/src/`, shared with the page | |
| `data.ts` (`useControlPlaneData`, `fetchControlPlaneAsks`, `mergeControlPlaneQuestions`) and its tests | | | `control-plane/src/read-model.ts` (§2); `useControlPlaneOverview` in the page |
| `use-summary.ts` and its tests | | | `control-plane/src/writer.ts` (§4) |
| `DecisionSideChat.tsx`, and the fork and metadata of `side-chat.ts` | `ade/web/src/components/chat/decision/`, behind §5; their imports of `./model`, `./answer-state` and `./NeedsYou` become the request's fields | | |
| `lib/decision-side-chat-lifetime.ts`, and the side-chat changes in `chat-aside.ts`, `conversations-context.tsx`, `ChatPanel`, `ChatView` | as they are | | |
| `kits/control-plane/result.schema.json`, `kits/control-plane/tests/test_contract.py` | | `control-plane/src/result.schema.json` (bundled) and `control-plane/tests/` | |
| The prototype’s built-in screen (`workspace-tabs.ts`, `App.tsx`, nav, palette, hash route) | replaced by the worker page | | |

### Import replacements in the moved page

| ADE-only | Replacement |
|---|---|
| `@/lib/iii-client`, `@/lib/backend` | `host.iii` |
| `@/lib/panel-context`, `@/lib/workspace-tabs` (`requestScreenOpen`, `chatSessionScreen`) | `host.panels.open` and `host.panels.openScreen`; Open session uses `host.chat.openConversation` |
| `@/hooks/use-media-query` | `useContainerNarrow` |
| `@/lib/sessions/types`, `@/types/chat` | the worker's types (`control-plane/src/types.ts`) |
| `@/lib/token-estimate`, `@/lib/conversations-context` (context size) | the read model's `context_size` (§2) |
| `@/lib/chat-aside` (git remotes) | none: PRs need `Project.repos` (§2) |
| `@/components/ui/*` | `@iii-dev/console-ui` (see below) |

What `@iii-dev/console-ui` covers:
- It exports `SegmentedControl` and `Card`.
- It does not export `TooltipProvider`, which only the tests use, or
  `formatTokenCount`, which the page keeps locally.
- It has no runtime, so the moved UI tests mock it with `vi.mock`, as
  `browser/ui/src/page/downloads.test.tsx` does.

## Design

### 1. The worker

`control-plane/` holds:
- `iii.worker.yaml` (`language: javascript`, `deploy: bundle`);
- `src/` in TypeScript, bundled with the schema to `dist/bundle`;
- `ui/`, built with `packages/console-ui/build-worker-ui.mjs`;
- `skills/SKILL.md`, copied into `dist/` at build;
- `tests/` and a README.

The deploy compiler accepts only explicit files or paths under `dist/`
(`.github/scripts/deployment_compiler.py:449-451`). The `include` in
`.deploy/workers.yaml` therefore lists files one by one.

Required workers: harness, session-manager, kanban, state and iii-directory
(D6).
GitHub is optional. The page shows PRs as unavailable when the worker is
absent. The worker does not need `shell`.

Configuration, under id `control-plane` in the configuration worker:

| Key | Default | Meaning |
|---|---|---|
| `stall_after_min` | 30 | `README.md` §7 |
| `summary.model` | none | `provider::model` of the writer, chosen in the settings with the chat's model picker; without it the summary is off |
| `summary.interval_min` | 10 | the writer's schedule, and the Refresh rate limit |
| `summary.language` | English | the writer's prose language |
| `prs.watched_interval_min`, `prs.idle_interval_min` | 5, 30 | GitHub cadence for a repository (§2) |
| `escalate` | `{}` | per project id, the question classes that reach the person as Now (§7); unset means `[security, publish, external_cost, data]` |

### 2. The read model

These functions read only:

| Function | Returns |
|---|---|
| `control-plane::overview {scope}` | direction (§7), tiles, the top Now questions and stalled runs, exceptions, runs in progress, PRs, the board, the latest valid summary with its freshness, and `availability: { github: boolean }` |
| `control-plane::needs-you::list {scope}` | questions (with `escalate` applied) and stalled runs, in `README.md` §6.1 order |
| `control-plane::runs::list {scope}` | `README.md` §7, `context_size` per root |
| `control-plane::prs::list {scope}` | open PRs of `Project.repos` with checks, card, session and wait reason |
| `control-plane::decided::list {scope, cursor?}`, `control-plane::assumptions::list {scope, since?}` | from `kanban::question::list`; `since` is filtered in the worker by the comment's `created_at` |
| `control-plane::watch {scope}` | marks the scope as watched for 2 min |
| `control-plane::summary::refresh {scope}` | §4 |

`scope` is `all` or a project id.

**Who may read.** A `control-plane::*` read returns only data from these sources:
- `harness::ask::list`;
- `session::list` and `session::messages-tail`;
- `harness::status`;
- kanban reads;
- `github::pr::*`.

Granting `control-plane::*` therefore grants, in effect, the union of those
sources. A policy that allows `*` exposes nothing new.
A narrowed policy names `control-plane::*` only where all of those sources are
already allowed.

**Sources.** The worker observes:
- `harness::on-ask` and `harness::on-ask-settled`;
- `harness::on-turn-started` and `harness::on-turn-completed` for roots
  (depth 0 or no parent, `harness/src/events.rs:53-96`);
- `kanban::on-change`.

Excluded from the read model and the hash: sessions with
`metadata.origin: control-plane`, and every side chat (`metadata.side_chat_of`), as
`data.ts:414` does today. Without that, each writer turn would change the
hash, and the writer would run every interval. The writer still observes its
own sessions (§4).

**Recompute.**
- The worker keeps one in-memory result per scope.
- It rebuilds a scope on source events, with a 2 s debounce.
- A deadline timer also rebuilds at the next time boundary: a run crossing
  `stall_after_min`, a run leaving the 24 h window, or a summary going
  stale. It fires at least once a minute while a run is live, because a run
  that stops emits nothing.
- After a rebuild that changed something, the worker emits
  `control-plane::on-changed {scope, as_of, parts}`.
- `ponytail:` one process, in-memory cache; a restart rebuilds it.

**`context_size`.**
- It comes from `harness::status {verbose: true}` `context.total`, cached
  by `(turn_id, step)`.
- When `context` is absent, the status is null or it has no `turn_id`, the
  worker falls back to an estimate from `session::messages-tail`.

**PRs.**
- They come only from `Project.repos`; a project without repos shows the
  existing "add repositories" state.
- The cache is per repository, shared by every scope, so `all` and a
  project never read the same PR twice.
- A repository is read every `prs.watched_interval_min` while a scope that
  contains it is watched.
- Otherwise it is read every `prs.idle_interval_min` while one of its
  projects has a live run, and not at all when neither holds.
- A `control-plane::watch` on a scope whose repositories were last read more than
  `prs.watched_interval_min` ago reads them at once.
- Each PR carries `as_of`. The page shows it when it is older than the
  interval.

**The page side (`control-plane/ui`).** `useControlPlaneOverview`:
- calls `control-plane::overview` and the other reads;
- subscribes once to `control-plane::on-changed` and refetches the parts that
  changed;
- calls `control-plane::watch` every minute while open.

### 3. Actions

As today, the page calls the target functions directly, as the console
(D5):

| Action | Call |
|---|---|
| Answer, Let the agent decide | `harness::ask::answer` with `by: {channel: "control-plane", user}` |
| Stalled run: Resume | `harness::send` "continue" to the root |
| Reopen an assumption | `kanban::comment::create` |
| Refresh the summary | `control-plane::summary::refresh {scope}` |

### 4. The summary writer

**The session.** One per scope: `control-plane-<scope>`, kind `automation`,
`metadata.origin: control-plane`. The worker ensures it with `session::ensure` and
reads `harness::status {verbose: true}` before each send.

**What each send carries.**
- When the status is null or has no `turn_id`, the session never ran, and
  the send carries `options.agent: "control-plane-writer"`. `agent` is accepted
  only when a session starts (`harness/src/functions/send.rs:1791-1799`).
  Later sends carry no `agent`.
- Every send carries:
  - `model` and `provider` at the top level, split from `summary.model`;
  - `options.functions.allow` with the reads listed in `README.md` §3;
  - `options.output` with the bundled `result.schema.json`.

**Adoption after a restart.** An existing `control-plane-<scope>` is adopted when
its kind is `automation`, its origin is `control-plane`, and either:
- it has no turn yet, or
- its `metadata.agent_profile.id` is `control-plane-writer`.

Otherwise the worker reports the scope as unavailable.

**Profile missing.** If `directory::agents::get {id: "control-plane-writer"}` fails,
nothing is sent. The scope shows "writer profile missing: apply the control-plane
kit", not "update failed".

**When to send.**
- Every `summary.interval_min`, the worker checks each scope that was
  watched in the last 2 min or has a live run.
- A check computes a source hash over:
  - changed cards' ids and `updated_at`;
  - completed root turn ids, except excluded sessions (§2);
  - PR heads and check states;
  - open ask ids and states.
- Per scope, the worker stores in `state` (scope `control-plane`, key
  `summary:<scope>`), on its own clock:
  - the last valid result;
  - `valid_hash`, the hash that result covers;
  - `checked_at`, when it last found the sources still equal to
    `valid_hash`.
- When the hash equals `valid_hash`, nothing is sent, and `checked_at`
  advances.
- When the hash differs, the worker sends.
  - The result arrives through the worker's own `harness::on-turn-completed`
    observer on `control-plane-<scope>` sessions. The §2 exclusion does not apply
    to it.
  - `valid_hash` changes only when that result passes the `summary.ts`
    rules.
- When the turn fails or the result is invalid, `valid_hash` stays, and the
  next check sends again.
  - The Overview shows the last valid summary with its own age and "update
    failed", never as fresh.
  - After 3 failures in a row, the interval doubles with each further
    failure, up to 8 × `summary.interval_min`.
  - A valid result resets the interval.
- The freshness on screen is `checked_at`, so a quiet project never looks
  stale.
- A per-scope mutex serializes checks, Refresh and session creation.

**Refresh.** It sends whatever the hash, at most once per scope per
`summary.interval_min`. Inside that window it returns `{throttled_until}`.
A binding or an agent can therefore spend at most one writer turn per
interval.

**Keys.**
- `control-plane:<scope>:<slot>` for scheduled sends.
- `control-plane:<scope>:<slot>:refresh` for Refresh, unique per slot because of
  the rate limit.

**Boot.** The worker reads `summary:<scope>` back from `state`. A restart
after a failed turn keeps the last valid summary, and sends nothing while the
hash is unchanged.

**No model configured.** Nothing is sent, and the Overview shows "set the
summary model in the Control plane settings".

### 5. Host capability for the side chat

The control plane sends the question form of the ADE host request.

```ts
host.chat.openDecisionChat?(request): Promise<handle>     // creates or reuses the copy
host.chat.rebindDecisionChat?(request): Promise<handle | null>  // existing copy only; never forks, never opens

request: {
  key: string;                      // `${sessionId}/${questionId}`
  sessionId: string;
  kind: 'question';
  title: string;                    // the copy's title
  agent?: string;                   // asked_by
  card?: { key: string; title: string };
  board?: { ticket: string; createdAt: string };   // the question's board trail
  sourceTitle?: string;             // title of the asking session, for the fallback quote
  createdAt: string;
  why?: string;
  questionId: string;
  questions: { header: string; question: string; multi_select?: boolean;
               options: { label: string; description?: string; recommended?: boolean }[] }[];  // 1–4
  onUseAnswer(questionIndex: number, proposal: { option?: string; text?: string }): void;
}
handle: {
  update(state: { editable: boolean; closed: boolean }): void;
  detach(): void;                   // on unmount; "Use this answer" is disabled until a rebind
  close(): void;
}
```

**The quote.** The host uses the assistant text of the step that asked (found
at the fork point), then the questions, the options with their descriptions,
`why` and the board trail. When the fork point has no assistant text,
`sourceTitle` stands in.

**Open or closed.** A chat is open while a `chat:<copy>` column stands
beside the page in the workspace tab layout. The layout is shared, so
closing the chat in one browser closes it everywhere, as with any column.
- `rebindDecisionChat` never adds a column.
- On mount, and when its list changes, the page calls `rebindDecisionChat`
  with the full request for every question it shows. The host binds each copy
  that has a column, so after a reload "Use this answer" works for any open
  copy, not only the selected one.
- A copy with no bound page has "Use this answer" disabled.
- "Ask about this" calls `openDecisionChat`.

**Where it opens.** In the ADE's chat side panel of the current workspace
tab, beside the caller. On a phone, full screen.

**What the ADE owns.** The fork point, metadata, Keep and cleanup
(`lib/decision-side-chat-lifetime.ts`, started from
`conversations-context.tsx:279`).

**Older ADE.** Without the capability, the page hides "Ask about this".

### 6. ADE changes

- The worker page opens at `#/worker/control-plane`. The workspace opens
  `ext:control-plane` via `#/workspace/control-plane`, including when it restores
  an old pane or redirects an old bookmark.
- `DecisionSideChat` and the host part of `side-chat.ts` move under
  `ade/web/src/components/chat/decision/` and back §5.

### 7. The Overview follows the canvas

The design canvas is the source of truth for the page. The Overview has:
- **Summary:** a box with the eyebrow and the writer's age, the headline,
  and three natural-language parts side by side (stacked when narrow): what
  happened, what is happening now, and what is left, with references linked
  in place. The settings' `summary.guidance` steers focus, tone and length. Without a model or the
  writer profile, the canvas no-summary panel, with Open settings.
- **Five tiles:** Needs you (now / no rush), Working, Waiting (on you /
  stalled), Open PRs (CI failing in N), Active releases.
- **Left column:**
  - Needs you: the Now items with their class, card, age, question,
    "Recommended:" option, Open and Ask about this, then No rush in one row;
  - Diary: today's milestones.
- **Right column:**
  - Releases (released, now and next, with what waits on you);
  - In progress (root sessions);
  - Pull requests (why each one waits);
  - Board (open columns, then Backlog and Done folded).

Now holds stalled runs and questions of a class in the project's
`escalate` list; everything else is No rush.

The other tabs use the same panels:
- **Needs you:** each row shows the asker, the card and what it blocks,
  with the age and the kind as a pill on the right. Now is sorted by the
  number of cards blocked. The Answer action follows the form.
- **Runs:** one panel per run, with the panel header band and the
  facts in the canvas's two columns. A stalled run's callout is red.
- **Assumptions and Decided:** one panel with rows split by a line,
  and the outcome as a pill.

**Withdrawn (2026-11-23):**
- the Direction block and the at-risk rule (release dates are not
  monitored; `at_risk_days` and the writer's `direction` are gone);
- the Exceptions block;
- "Decided for you since your last visit" and its per-browser last visit.

**Kept:** the Decided tab's header shows the median time a decision waited
over the last 7 days, from `asked_at` to `settled_at` of closed entries
(`harness/src/ask_store.rs`).

**Settings:** `summary.model` is chosen with the chat's `ModelPicker`, over
the same `router::models::list` catalog, stored as `provider::id`.

## Migration (new commits on #1855)

1. **Worker:** `control-plane/`, with:
   - the read model and the deadline timer;
   - the writer and the bundled schema;
   - the tests from `kits/control-plane`, which is cut down (D6).
2. **Page:** `control-plane/ui/`, with the moved components, `useControlPlaneOverview`,
   direct console actions and the canvas layout.
3. **ADE:** the built-in screen is removed, `openDecisionChat` and
   `rebindDecisionChat` are added, and old bookmarks redirect to
   `ext:control-plane`.
4. **Docs:** `README.md` §1 and §3 point to the worker, and
   `.deploy/workers.yaml` gains the `control-plane` entry.
5. **Verification:** see Validation.

## Validation

1. **Parity.**
   - `model.test.ts` and `summary.test.ts` pass in `control-plane/`.
   - For the Control plane fixtures, the worker computes question and run
     states, counts and ordering. PR repositories come from `Project.repos`.
2. **Time.** A root that stops emitting becomes a stalled run within a
   minute of crossing `stall_after_min`.
3. **Writer.**
   - With no model, nothing is sent.
   - The first send carries `options.agent`, judged by status, and later
     sends do not.
   - A session left between ensure and the first send is adopted.
   - An unchanged hash sends nothing and advances `checked_at`.
   - A failed or invalid result is sent again at the next check, and the
     Overview shows "update failed".
   - The writer's own turns do not change the hash.
   - A second Refresh inside the interval is throttled.
   - Three invalid results in a row back the interval off, and a valid one
     resets it.
   - A restart after a failed turn keeps the last valid summary and sends
     nothing.
   - Without the kit, the scope shows "writer profile missing".
4. **Profile.** After `iii call compose::add worker=control-plane` and applying the
   `control-plane` kit:
   - `directory::agents::get {id: "control-plane-writer"}` resolves;
   - the first send succeeds;
   - applying the kit does not add `github`.
5. **Safety.**
   - No `control-plane::*` function acts for a person.
   - A binding targeting `control-plane::summary::refresh` spends at most one turn
     per interval.
6. **Load.**
   - Three tabs on one scope, plus one on `all`: GitHub is read once per PR
     per interval.
   - With no tab open and no live run, nothing is read.
   - The first watch after that reads at once and shows `as_of`.
7. **Side chat.**
   - There is one copy per key.
   - After a reload, `rebindDecisionChat` forks nothing, and "Use this
     answer" fills the right question in every open copy.
   - A closed chat stays closed.
   - `detach` disables "Use this answer" until a rebind.
   - An older ADE hides the button.
8. **Canvas.**
   - The Overview lays out as §7, with no Direction, Exceptions or release
     dates.
   - An escalated class is Now, and others are No rush.
   - The settings pick `summary.model` from the chat's catalog.
9. **Regression.**
   - The harness direct integration suite stays at 41 of 41.
   - The ade/web and kanban suites pass.
   - Fixture captures match the canvas at 390, 720 and 1440 px, in both
     themes.

## Open decisions

- **D1–D6:** see Decisions.
- **Last visit:** kept per browser until the ADE has a viewer identity.
