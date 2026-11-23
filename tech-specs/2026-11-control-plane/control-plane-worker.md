# Control plane worker

Status: proposed, 2026-11-22. Revision 5, after four adversarial reviews.

This spec extends `README.md`, the control plane spec, in PR #1855.

It moves the control plane out of the ADE shell into a `control-plane` worker, which owns:
- the screen;
- a read model;
- the schedule of the summary writer.

It also orders the Overview for management by exception (the CEO view,
§7).

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
  The fork, its metadata, the approval allows and the cleanup are chat
  infrastructure.
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
  agent to `harness::ask::answer` or `approval::*`.
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
| `DecisionSideChat.tsx`, and the fork, metadata and allows of `side-chat.ts` | `ade/web/src/components/chat/decision/`, behind §5; their imports of `./model`, `./answer-state` and `./NeedsYou` become the request's fields | | |
| `lib/decision-side-chat-lifetime.ts`, and the side-chat changes in `chat-aside.ts`, `conversations-context.tsx`, `ChatPanel`, `ChatView` | as they are | | |
| `kits/control-plane/result.schema.json`, `kits/control-plane/tests/test_contract.py`, `kits/control-plane/iii-permissions.yaml` | | `control-plane/src/result.schema.json` (bundled) and `control-plane/tests/`; the permissions become the writer's `options.functions` (§4) | |
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
Optional: approval-gate and github; the page shows their part as
unavailable when one is absent. The worker does not need `shell`.

Configuration, under id `control-plane` in the configuration worker:

| Key | Default | Meaning |
|---|---|---|
| `stall_after_min` | 30 | `README.md` §7 |
| `summary.model` | none | `provider::model` of the writer; without it the summary is off |
| `summary.interval_min` | 10 | the writer's schedule, and the Refresh rate limit |
| `summary.language` | English | the writer's prose language |
| `prs.watched_interval_min`, `prs.idle_interval_min` | 5, 30 | GitHub cadence for a repository (§2) |
| `escalate` | `{}` | per project id, the question classes that reach the person as Now (§7); unset means `[security, publish, external_cost, data]` |
| `at_risk_days` | 3 | §7 |

### 2. The read model

These functions read only:

| Function | Returns |
|---|---|
| `control-plane::overview {scope}` | direction (§7), tiles, the top Now questions and stalled runs, exceptions, runs in progress, PRs, the board, the latest valid summary with its freshness |
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

Held calls are left out. Granting `control-plane::*` therefore grants, in effect,
the union of those sources: a policy that allows `*` exposes nothing new.
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
- reads held calls itself, as console calls (`approval::list-pending`,
  `approval::on-pending-created`, `approval::on-pending-resolved`);
- calls `control-plane::watch` every minute while open.

### 3. Actions

As today, the page calls the target functions directly, as the console
(D5):

| Action | Call |
|---|---|
| Answer, Let the agent decide | `harness::ask::answer` with `by: {channel: "control-plane", user}` |
| Held call: Deny / Allow once / Always allow | `approval::resolve`; for Always, `approval::approve-always` first |
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
  - a read-only `options.functions`, the list that was
    `kits/control-plane/iii-permissions.yaml`;
  - `options.output` with the bundled `result.schema.json`.

**Adoption after a restart.** An existing `control-plane-<scope>` is adopted when
its kind is `automation`, its origin is `control-plane`, and either:
- it has no turn yet, or
- its `metadata.agent_profile.id` is `control-plane-writer`.

Otherwise the worker reports the scope as unavailable.

**Approvals.** On creation or adoption, the worker sets the session to
approval auto mode, with always-allow for exactly its reads. When
approval-gate is absent, it skips this, because nothing would hold the reads.

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

```ts
host.chat.openDecisionChat?(request): Promise<handle>     // creates or reuses the copy
host.chat.rebindDecisionChat?(request): Promise<handle | null>  // existing copy only; never forks, never opens

request: {
  key: string;                      // `${sessionId}/${questionId}` or `${sessionId}/${functionCallId}`
  sessionId: string;
  kind: 'question' | 'held';
  title: string;                    // the copy's title
  agent?: string;                   // asked_by
  card?: { key: string; title: string };
  board?: { ticket: string; createdAt: string };   // the question's board trail
  sourceTitle?: string;             // title of the asking session, for the fallback quote
  createdAt: string;
  why?: string;
  // kind 'question'
  questionId?: string;
  questions?: { header: string; question: string;
                options: { label: string; description?: string; recommended?: boolean }[] }[];  // 1–4
  // kind 'held'
  held?: { functionCallId: string; functionId: string; arguments: unknown };  // arguments already redacted
  onUseAnswer(questionIndex: number, proposal: { option?: string; text?: string }): void;
}
handle: {
  update(state: { editable: boolean; closed: boolean }): void;
  detach(): void;                   // on unmount; "Use this answer" is disabled until a rebind
  close(): void;
}
```

**The quote.** The host composes it.
- For a question: the assistant text of the step that asked (found at the
  fork point when the copy is created), then the questions, the options
  with their descriptions, `why` and the board trail. When the fork point
  has no assistant text, `sourceTitle` stands in.
- For a held call: the function id and its redacted arguments.

The page never builds quote text.

**Use this answer.** For a held call it proposes Deny, Allow once or Always
allow, mapped to `questionIndex` 0.

**Open or closed.** A chat is open while a `chat:<copy>` column stands
beside the page in the workspace tab layout. The layout is shared, so
closing the chat in one browser closes it everywhere, as with any column.
- `rebindDecisionChat` never adds a column.
- On mount, and when its list changes, the page calls `rebindDecisionChat`
  with the full request for every item it shows. The host binds each copy
  that has a column, so after a reload "Use this answer" works for any open
  copy, not only the selected one.
- A copy with no bound page has "Use this answer" disabled.
- "Ask about this" calls `openDecisionChat`.

**Where it opens.** In the ADE's chat side panel of the current workspace
tab, beside the caller. On a phone, full screen.

**What the ADE keeps.** The fork point, the metadata, the approval allows,
Keep and cleanup (`lib/decision-side-chat-lifetime.ts`, started from
`conversations-context.tsx:279`).

**Older ADE.** Without the capability, the page hides "Ask about this".

### 6. ADE changes

- The worker page opens at `#/worker/control-plane`. The workspace opens
  `ext:control-plane` via `#/workspace/control-plane`, including when it restores
  an old pane or redirects an old bookmark.
- `DecisionSideChat` and the host part of `side-chat.ts` move under
  `ade/web/src/components/chat/decision/` and back §5.

### 7. The CEO view

The Overview is ordered for management by exception:

1. **Direction.** Releases with a `target_at`, each on track or at risk.
   - A release is at risk when:
     - its date has passed;
     - its date falls within `at_risk_days` with cards still open
       (`kanban::release::list` `done`/`total`);
     - one of its questions of an escalated class is open.
   - The class comes from the open entries of `harness::ask::list`
     (`about.class`), which the read model already holds.
     `kanban::release::list` `open_questions` is a count without class.
   - The writer's result gains `direction`: one line per release, with
     references, under `README.md` §3's prose rules.
2. **Needs you.**
   - Now holds questions of a class in the project's `escalate` list, held
     calls (always) and stalled runs.
   - Everything else is No rush.
3. **Exceptions:**
   - stalled runs;
   - PRs with CI failing;
   - cards labelled `blocked`;
   - questions open for more than a day.
4. **Decided for you.** Assumptions recorded since the viewer's last visit
   (`control-plane::assumptions::list {since}`).
   - The last visit is kept per browser (`usePaneState`).
   - `ponytail:` per browser, not per person; the ADE has no viewer
     identity.
5. **The rest, folded:** releases and runs in full, PRs, the board, the
   diary.

The Decided header shows the median time a decision waited over the last 7
days, from `asked_at` to `settled_at` of closed entries
(`harness/src/ask_store.rs`). If it grows, the `escalate` list is too wide.

## Migration (new commits on #1855)

1. **Worker:** `control-plane/`, with:
   - the read model and the deadline timer;
   - the writer and the bundled schema;
   - the tests from `kits/control-plane`, which is cut down (D6).
2. **Page:** `control-plane/ui/`, with the moved components, `useControlPlaneOverview`,
   direct console actions and the CEO order.
3. **ADE:** the built-in screen is removed, `openDecisionChat` and
   `rebindDecisionChat` are added, and old bookmarks redirect to
   `ext:control-plane`.
4. **Docs:** `README.md` §1 and §3 point to the worker, and
   `.deploy/workers.yaml` gains the `control-plane` entry.
5. **Verification:** see Validation.

## Validation

1. **Parity.**
   - `model.test.ts` and `summary.test.ts` pass in `control-plane/`.
   - For the Control plane fixtures, the read model returns what the ADE page
     computed, with two expected differences:
     - held calls are absent, because the page reads them;
     - the git-remote fallback for PRs is gone.
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
   - The writer's reads are not held.
   - Three invalid results in a row back the interval off, and a valid one
     resets it.
   - A restart after a failed turn keeps the last valid summary and sends
     nothing.
   - Without the kit, the scope shows "writer profile missing".
   - Without approval-gate, the session is created and sends.
4. **Profile.** After `iii call compose::add worker=control-plane` and applying the
   `control-plane` kit:
   - `directory::agents::get {id: "control-plane-writer"}` resolves;
   - the first send succeeds;
   - applying the kit does not add `github`.
5. **Safety.**
   - No `control-plane::*` function acts for a person.
   - `control-plane::*` responses carry no held-call data.
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
   - A held call quotes its function and its redacted arguments.
   - An older ADE hides the button.
8. **CEO view.**
   - A release past its date with open cards is at risk.
   - So is a release with an open question of an escalated class.
   - An escalated class is Now, and others are No rush.
   - Decided for you lists only assumptions after the last visit.
9. **Regression.**
   - The harness direct integration suite stays at 41 of 41.
   - The ade/web and kanban suites pass.
   - Fixture captures match the canvas at 390, 720 and 1440 px, in both
     themes.

## Open decisions

- **D1–D6:** see Decisions.
- **Last visit:** kept per browser until the ADE has a viewer identity.
