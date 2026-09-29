# chat-app

A base scaffold for a chat surface, built with Vite + React + TypeScript +
Tailwind v4 and styled to the iii Schematic design system
(see [`../skills/design-system.md`](../skills/design-system.md) for the full spec).

It runs entirely client-side with mocked streaming, so there are no API keys
to configure. The mock backend ships in dev (`pnpm dev`) and is tree-shaken
out of production builds. Every component variant and every streaming edge
case (errors, aborts, multi-function runs, long markdown, …) lives in
Storybook (`pnpm storybook`). Drop a real provider in by replacing one file
(see [Swapping in a real backend](#swapping-in-a-real-backend) below) and
[`PLAYGROUND.md`](./PLAYGROUND.md) is the streaming contract you have to honor.

## Quickstart

```bash
npm install
npm run dev
```

Then open the printed `Local:` URL (Vite picks the first free port from
5173 upwards).

## Keeping the mobile screen awake

The console uses the browser's Screen Wake Lock API while the document is
visible and there is active work:

- A connected session is `working` (model wait, reasoning, streamed replies,
  tool calls and in-turn approvals), including subagents and other workspace
  panels. The shared conversation provider owns this, not the selected pane.
- A local send/queued-message edit is still preparing or uploading attachments,
  submitting, streaming, or compacting a session with `/compact`.
- Voice dictation is starting, listening, or flushing its final transcription.

Concurrent activities share a lock; completion, errors and cancellation release
their own leases. Idle/done/error sessions, unsent drafts, armed future triggers,
background catalog/trace refreshes and an open WebSocket do not keep it awake.
Audio playback alone is not a reason to force the screen on.

The lock is released when hidden and reacquired when visible if work remains.
This requires browser support and a secure context (normally HTTPS); denial or
battery-saving restrictions degrade silently. It does **not** prevent manual
locking, override OS policy, or guarantee execution in the background.

Injected UI can opt in for finite foreground work with
`const release = host.screen?.keepAwake()`, calling `release?.()` on every
completion/error/cancel path. Leases are also cleaned up on script disposal.
Feature-detect this optional API for older consoles; do not acquire merely
because a page is mounted or a subscription is registered.

Manual device check: install/open the PWA over HTTPS, start a turn longer than
the phone's auto-lock timeout (also try a slow tool, subagent and voice dictation),
and verify the screen stays on. Switch away and back during work, then confirm
normal auto-lock resumes after all activity finishes or is cancelled.

## Scripts

| command            | what it does                              |
| ------------------ | ----------------------------------------- |
| `npm run dev`      | Start the Vite dev server with HMR.       |
| `npm run build`    | Type-check, then build a static bundle.   |
| `npm run preview`  | Serve the built bundle locally.           |
| `npm run typecheck`| Type-check without emitting.              |
| `npm run storybook`| Component + scenario stories on `:6006`.  |
| `npm run build-storybook` | Build the static Storybook site.   |

## What's in the box

- **Composer** powered by [`lexical`](https://lexical.dev/) in plain-text
  mode. The plugin layer is intentionally thin so that autocomplete, mention
  pickers, or slash menus can be added later without restructuring the
  editor.
- **Markdown** rendering via `react-markdown` + `remark-gfm`, with element
  renderers that follow the iii Schematic (natural-case sans headings and
  prose, with monospace reserved for code and technical values).
- **Backend seam** in [`src/lib/backend/`](src/lib/backend/). `ChatView`
  consumes a `ChatBackend` interface that yields a documented stream of
  events; the mock (dev) and the stub real backend (prod) both implement it.
  Three canned bodies — one per mode — exercise headings, lists, fenced
  code, blockquotes, and inline code on the first run.
  See [`PLAYGROUND.md`](./PLAYGROUND.md) for the full contract.
- **Storybook** (`pnpm storybook`) — the component spec sheet (composer,
  messages, function views, shared settings primitives, type, color) plus the
  streaming **playground**: a chat surface driven by a catalog
  of scenarios (errors, aborts, multi-function runs, slow/fast streams, long
  markdown) that stress every corner of the streaming contract. Useful before
  swapping in a real backend.
- **Shared UI recipes** keep content tabs on the line variant with default
  16 px icons, neutral selection, natural-case sans labels, and shared
  responsive motion across native and injected worker surfaces.
- **Model picker** and **mode picker** (`ask` / `agent`) wired into
  the canned response so you can see the values flow through.
- **File attachments** via a hidden file input. Previewable text/image
  files store a data URL; binaries store metadata only. Attachments are
  cleared after the next outgoing message.
- **Sidebar** listing conversations backed by the
  [session-manager](../../session-manager/architecture/integration.md)
  worker (`session::list`; live via the `session::*` trigger types).
  Transcripts hydrate from `session::messages` and stream live from
  `session::message-added` / `session::message-updated` snapshots —
  localStorage keeps only UI affordances (active id, last model).
  Double-click a row to rename inline (writes through `session::set-meta`);
  hover to reveal the delete affordance (`session::delete`).
- **Worktree surface** backed by the optional
  [worktree](../../worktree) worker: a worktrees tab in the
  working-directory picker (picking validates the path and claims the
  worktree for the session, released when the conversation points
  elsewhere), a working-dir badge with branch / dirty / ahead / lifecycle,
  live landed and land-blocked notices in chat, and the `#/worktrees`
  graph page. Every piece is gated on worker presence.
- **Light / dark theme** toggle, persisted under `iii-theme` and applied
  pre-paint to avoid a flash.

## Layout

```
src/
  main.tsx
  App.tsx                # routing (traces + configuration + worktrees) + always-on chat dock
  index.css              # Tailwind v4 + iii Schematic tokens + utilities
  lib/
    utils.ts             # cn = twMerge(clsx(...))
    storage.ts           # localStorage for UI affordances (active id, last model)
    markdown.tsx         # iii-styled react-markdown wrapper
    worktrees.ts         # worktree worker wire types, calls, labels
    worktree-claims.ts   # console-made claims + auto-release decision
    sessions/            # session-manager integration
      api.ts             #     session::* calls (list/ensure/set_meta/delete/messages)
      events.ts          #     bindings for the six session::* trigger types
      entry-mapper.ts    #     SessionEntry → UI Message segments + reconcile
      types.ts           #     wire types (SessionMeta, TranscriptItem, events)
    backend/             # ← the seam. ChatBackend interface + impls
      types.ts           #     StreamEvent, ChatBackend, ChatStreamOptions
      harness-send.ts    #     harness::send / stop / status wire helpers
      turn-events-live.ts     # harness::turn-completed subscription
      approval-events-live.ts # approval::pending-* subscription + list-pending
      translate.ts       #     trigger payloads → StreamEvent
      real.ts            #     harness::send kickoff + turn/approval triggers
      index.ts           #     getDefaultBackend()
  types/chat.ts          # Conversation, Message, Mode, ModelId, Attachment
  hooks/
    use-conversations.ts # server-backed conversation store (session-manager)
    use-hash-route.ts    # #/traces #/configuration #/worktrees
    use-worktree-status.ts   # worker presence probe (gates the surface)
    use-worktree-binding.ts  # working dir -> managed worktree for the badge
    use-worktree-events.ts   # landed / land-blocked trigger bindings
    use-theme.ts         # theme + persistence
  components/
    ui/                  # iii Schematic primitives (+ co-located *.stories.tsx)
    sidebar/             # ConversationSidebar + ConversationRow
    chat/                # ChatView, Composer, Message, … (+ *.stories.tsx)
  pages/
    Configuration/       # global Settings + worker-owned config forms
    Traces/              # trace explorer
    Worktrees/           # live worktree graph (repo -> worktree -> session)
  stories/               # Storybook-only assets (never in the app bundle)
    fixtures/            # mock data for the component stories
    design/              # typography / color / loading token sheets
    playground/          # ChatView harness + EventLog + scenario stories
      scenarios/         # one ChatBackend per file
```

## Swapping in a real backend

Open [`src/lib/backend/real.ts`](src/lib/backend/real.ts) and replace the
stub generator with one that talks to your provider. The shape of the
events you yield is defined in
[`src/lib/backend/types.ts`](src/lib/backend/types.ts) and explained in
[`PLAYGROUND.md`](./PLAYGROUND.md). As long as your generator yields
`StreamEvent`s in the documented order, the chat surface and every
playground scenario keep working — that's the whole point of the seam.

A sketch for OpenAI's chat-completions stream:

```ts
import type { ChatBackend } from './types'

export const realBackend: ChatBackend = {
  id: 'openai',
  async *stream(prompt, _mode, model, opts) {
    const res = await fetch('https://api.openai.com/v1/chat/completions', {
      method: 'POST',
      signal: opts?.signal,
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${import.meta.env.VITE_OPENAI_API_KEY}`,
      },
      body: JSON.stringify({
        model,
        stream: true,
        messages: [{ role: 'user', content: prompt }],
      }),
    })
    // ... read res.body as a ReadableStream, parse SSE chunks,
    //     yield { kind: 'assistant-token', token } as each delta arrives,
    //     finish with { kind: 'assistant-end' }.
  },
}
```

To verify your implementation against the same edge cases the mock survives,
open Storybook (`pnpm storybook`) and walk every story under **Playground**.
If they all render correctly, your backend is contract-clean.

## Design system

[`../skills/design-system.md`](../skills/design-system.md) is the canonical design system
(tokens, typography, the numbers table, components, UX patterns); this directory's
`DESIGN.md` is only a pointer to it. Theme and motion tokens live in
[`src/index.css`](src/index.css); the public list, tree, card, card-highlight,
collapsible-card, panel, chip, table, tabs, field, switch, settings, eyebrow, toolbar,
status-bar and motion recipes live in
[`src/styles/ui-recipes.css`](src/styles/ui-recipes.css).

Shared behavior belongs in [`src/components/ui`](src/components/ui) and is
exported to injected workers through `@iii-dev/console-ui`. Prefer the shared
`PageShell`/`PageHeader`/`PageSidebar`, `Selector`, `Tooltip`, `SegmentedControl`, `List`,
`Card`, `CollapsibleCard`, `CardHighlight`, `Panel`, `Chip`, `IconButton`, `Eyebrow`,
`SearchField`, `Toolbar`/`StatusBar` and `useConfirm` contracts over local copies; the
bundleable `@iii-dev/console-ui/hooks` (`useContainerNarrow`, `usePaneState`,
`useCopyFlash`, `useWorkerLive`) and `@iii-dev/console-ui/format` cover the behaviour a
worker page would otherwise re-implement. Selection is neutral in light and
dark themes; reserve accent for primary actions, form focus, live activity,
and semantic domain data. Update the canonical design guide, public
declarations/manifests, stories, and conformance tests when extending this
surface.

Worker-owned renderers have focused authoring guides:

- [`docs/custom-function-components.md`](docs/custom-function-components.md)
  for function invocation request/result UI;
- [`docs/custom-trigger-components.md`](docs/custom-trigger-components.md)
  for source-specific trigger registration, firing, and retirement UI.
