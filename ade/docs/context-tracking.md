# Chat-owned Context tracking

Users ask in chat to keep release progress, a goal, status or a checklist in the
Context sidebar. They do not write code or install a custom extension per item.
The agent discovers `console::context::{list,save,remove}` and stores a structured
snapshot. `save` creates without an id; with an existing visible id it replaces
all fields and may move scope. The Harness pre-trigger hook stamps the current
session, overriding an agent-supplied session id. A pre-generation hook explains
the feature and lists existing ids, without promoting stored prose to system
instructions. No changes to Harness itself are required.

Examples:

- “Track release v1.8 in Context.” — conversation scope, the default.
- “Show this tracking in all conversations for this project.” — move the same id
  to project scope, only after an explicit request.
- “Update the release progress.” — read the source, then replace the snapshot.
- “Remove that tracking from Context.” — remove the existing id.

The source must be consulted before claiming current numeric progress. Failed
refreshes retain the previous snapshot. There is no background polling, automatic
source adapter or scheduled agent turn. The UI re-reads saved snapshots when the
panel opens, a turn ends, focus returns, the session/project changes or the engine
reconnects. Update/Remove buttons prepare a chat draft and do not send it.

## Persistence and scope

`<console.data_dir>/context-tracking.json` contains snapshots (not authoritative
release records). Writes use a shared mutation lock and atomic rename. Corrupt
files fail visibly rather than being overwritten. There are at most 100 items per
scope, with bounded title, summary, checklist and source fields. Only absolute
HTTP(S) source URLs without credentials are accepted. Text is rendered as text,
not HTML. Numeric progress is finite and in 0..100.

Conversation scope is keyed by session id. Project scope is keyed by the existing
session's canonical `metadata.fs_scope.root`, never an arbitrary project path in
the tool payload. Conversations using the same canonical directory share these
snapshots; another directory/worktree does not. Moving a conversation to another
project does not copy its old project's snapshots. Missing/invalid project
folders cannot receive new project-scoped tracking. List/edit/remove are limited
to the selected conversation and its current project. This uses the Console's
existing trusted engine/session boundary; it does not introduce multi-user auth.

The Tracking section uses existing fold/hide/reorder preferences. It includes an
empty-state hint, loading and retry states, scope, progress, checklist, source,
last-updated time and an explicit non-live disclaimer. It retains cached snapshots
on read failure and does not display a previous conversation's pending response.

## Verification

```sh
cd ade/web
pnpm exec tsc -b --noEmit
pnpm exec tsc -p tsconfig.e2e.json
pnpm exec vitest run src/components/chat/ContextTracking.test.tsx \
  src/components/chat/ChatContextPanel.test.tsx \
  src/components/chat/ContextSectionsMenu.test.tsx \
  src/components/chat/ChatAside.test.tsx \
  src/lib/context-groups.test.ts src/lib/ui-slots.test.ts
pnpm build
cd ..
cargo test --lib
cargo build --bin ade
```

The opt-in Playwright test boots its own engine, real session-manager and built
Console, with temporary data and dynamically allocated ports. It does not use the
operator's engine or a model provider. It verifies public function dispatch,
conversation isolation, project sharing, guidance, real rendering at 1440/780/390
pixels, light/dark, reduced motion, horizontal overflow, draft actions, persistence
across a Console process restart and removal. It is not an LLM-behaviour eval.

```sh
cd ade/web
CONTEXT_ENGINE_BIN=/absolute/path/to/iii \
CONTEXT_SESSION_BIN=/absolute/path/to/session-manager \
CONTEXT_CONSOLE_BIN=/absolute/path/to/worktree/ade/target/debug/ade \
PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH=/absolute/path/to/chrome \
pnpm exec playwright test e2e/context-tracking.spec.ts
```
