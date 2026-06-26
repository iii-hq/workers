# Spec: tree view for the main chat + subagents

> **IMPLEMENTED (2026-06-26) — Option A (nested sidebar rows).** `parentId`/`depth`
> on `Conversation`, populated from `metadata.parent_session_id`/`depth` in
> `use-conversations` (list map + `meta-updated`). Pure `conversation-tree.ts`
> (`buildConversationTree`/`flattenConversationTree`, orphan→root, cycle-safe).
> `ConversationRow` indent + chevron toggle; `ConversationSidebar` renders the
> flattened tree with per-node collapse persisted to `localStorage`. Tree unit
> tests + full suite (780) green. **Frontend-only — no backend change.**
> Option B (xyflow canvas) remains the deferred v2. Open items below (§"Decision
> to confirm" = shipped option 1 "show all, grouped"; §2 edge-latency verify)
> still want a live confirmation.

## Goal

Show the relationship between a main (orchestrator) conversation and the
sub-agent sessions it spawned, as a hierarchy — instead of the current flat,
ungrouped conversation list where a parent and its children are
indistinguishable siblings.

## Key finding: the linkage already exists — frontend-only change

When the harness spawns a sub-agent it **already stamps the parent linkage into
the child session's metadata** (`harness/src/subagent.rs:143-158`):

```rust
let linkage = json!({
    "parent_session_id": p.session_id,
    "parent_turn_id":   p.turn_id,
    "function_call_id": p.function_call_id,
    "depth": depth,
});
session.create(None, linkage.as_ref())   // (or session.ensure for fork)
```

Child sessions are real sessions, so they come back from `session::list`
(unfiltered today — `api.ts:16-23`), and each carries
`metadata.parent_session_id` in its `SessionMeta`. **No session-manager change is
needed** — this supersedes the "add `parent_id` to the backend" idea from the
earlier research note; the field is already there under
`metadata.parent_session_id`.

> Don't confuse with session-manager's other parent fields:
> `SessionEntry.parent_id` is **entry-level** threading *within* a transcript, and
> `SessionMeta.forked_from` is for forks. Neither is the spawn link — the spawn
> link is `metadata.parent_session_id`.

## Two render options

### Option A — nested rows in the sidebar (recommended for v1)

Group children under their parent in the existing conversation list, indented,
with a collapse caret on any row that has children. Smallest change, lives where
the user already looks, reuses `ConversationRow`.

### Option B — flow-graph canvas (follow-up)

A node graph (main → subagents) using `@xyflow/react` + `dagre`, both already
installed and wired in `pages/Traces/components/{FlowView,TraceMap}.tsx`. Richer
"see the whole fan-out at once" view; more work; needs its own route/panel and a
node-click → select-conversation bridge. **Defer to v2** once the linkage is
flowing in the model (Option A proves the data path with far less code).

Rest of this spec details **Option A**.

## Changes (Option A)

### 1. Surface the linkage on `Conversation` — `types/chat.ts`

Add two optional fields:

```ts
/** Spawn parent session id (from child SessionMeta.metadata.parent_session_id). */
parentId?: string
/** Spawn depth (0 = root/orchestrator); from metadata.depth. */
depth?: number
```

### 2. Populate them — `hooks/use-conversations.ts`

`conversationFromMeta` (`:115`) currently reads only `model/mode/title_manual`
from `meta.metadata`. Also read:

```ts
const parentId = typeof md.parent_session_id === 'string' ? md.parent_session_id : undefined
const depth = typeof md.depth === 'number' ? md.depth : undefined
```

and set them on the returned `Conversation`. Do the same in the
`onMetaUpdated` handler (`:259`). The `onCreated` stub (`:245`) can't set
`parentId` — `SessionCreatedEvent` carries no metadata (`sessions/types.ts:89`).
That's fine: the edge attaches on the next `meta-updated` or re-list.
**Verify**: confirm whether the harness emits a `meta-updated` (or the child
shows up in a `session::list`) shortly after spawn so the edge appears within a
second or two. If not, extend `SessionCreatedEvent`/the create emit with
metadata — note it, don't block v1 on it.

### 3. Build the tree — new `lib/conversation-tree.ts` (pure, testable)

```ts
export interface ConvNode { conversation: Conversation; children: ConvNode[]; depth: number }
export function buildConversationTree(list: Conversation[]): ConvNode[]
```

- Index by id. For each conversation, attach to `parentId`'s children if the
  parent is present; else it's a root.
- **Orphans** (parent not in the loaded page / parent deleted): treat as a root
  so nothing ever disappears. (The list is one 200-row page, `order:
  updated_desc` — a parent could be off-page. Acceptable; flag in UI only if it
  matters.)
- **Cycle guard**: cap recursion by `depth` / a visited set so a malformed
  `parentId` can't infinite-loop the render.
- Sort roots by `updatedAt desc` (matches today); sort children by `createdAt
  asc` (spawn order reads naturally).

### 4. Render — `ConversationSidebar.tsx` + `ConversationRow.tsx`

- Sidebar maps `buildConversationTree(conversations)` and renders each root,
  recursing into `children`.
- `ConversationRow` gains:
  - `depth?: number` → left padding `pl-{3 + depth*3}` (indent).
  - `hasChildren` + `collapsed` + `onToggle` → a caret button before the title
    (reuse `components/ui/Caret.tsx`); clicking toggles that subtree, not select.
  - A subtle child marker (e.g. the `└` guide or a depth tint) so nesting reads
    even at depth 1.
- Per-node collapse state: a `Set<string>` of collapsed parent ids in
  `ConversationSidebar` (or `ChatPanel`), persisted to `localStorage`
  (`iii-chat-tree-collapsed`) like the other UI affordances. Default expanded.

### 5. Status roll-up (nice-to-have, cheap)

A collapsed parent should still signal a working/error child: when a parent row
is collapsed, OR the status dot from `ConversationRow` to show the
"strongest" status across its subtree (`working` > `error` > else). Reuse the
existing `StatusDot` tones. Skip if it complicates v1 — add when collapsed
subtrees hide activity in practice.

## Decision to confirm before building

**Which sessions appear in the list?** Today `session::list` is unfiltered, so
sub-agent sessions already show flat (current clutter). Options:

1. **Show all, grouped** (recommended): every session renders; children nest
   under parents; standalone sessions stay roots. Zero hiding, just structure.
2. **Console-rooted only**: hide trees whose root isn't a `surface: console`
   session. Cleaner list, but hides sub-agents started by non-console
   orchestrators (workflows, other surfaces) that the user may want to inspect.

Recommend (1) — it only *adds* structure to what's already shown, no behavior
regression.

## Tests

- `conversation-tree.test.ts`: flat list → roots+children; orphan → root; cycle
  → terminates; ordering (roots desc, children asc).
- `use-conversations` mapper: `metadata.parent_session_id`/`depth` land on the
  `Conversation` from both `conversationFromMeta` and `onMetaUpdated`.
- `ConversationRow` story at `depth: 1/2` with a caret + children count.

## Risks / notes

- **Edge latency on fresh spawn** — see §2; child may render as a root for a
  moment until the meta/list carries `parent_session_id`. Self-heals; verify the
  window is short.
- **Off-page parent** — orphan-as-root handling covers it; the only artifact is a
  child shown at root until its parent paginates in. Acceptable for v1.
- **Deep fan-out depth** — indent caps visually; rely on collapse. `dagre`/xyflow
  (Option B) is the real answer for wide/deep graphs — that's why it's the v2.

## Estimate

~120–180 lines (type fields + mapper + pure tree builder + recursive render +
collapse state + tests). 1–1.5 days. Independent of the resizable-sidebar spec;
they touch adjacent files (`ConversationSidebar`, `ChatPanel`) but don't
conflict — do the resize first if sequencing, it's smaller.
