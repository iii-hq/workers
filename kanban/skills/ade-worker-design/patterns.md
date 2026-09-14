---
name: console-ui-patterns
description: >-
  Concrete, reusable interaction and layout recipes for injected Console pages
  that manage records: boards with lanes, a record screen that opens as its
  own pane, activity timelines with threaded comments, chat cards for agent
  calls, creation modals, and settings forms. Companion to
  console-injectable-ui.md (the contract) and console-design.md (the tokens).
---

# Console UI patterns for record-shaped workers

Most worker UIs manage a collection of records (tickets, jobs, documents,
rows). These recipes encode the decisions that make such a UI read as native
to the Console. Every snippet uses `<worker>` for the worker name; scope
every selector under `[data-iii-ui="<worker>"]` and use only tokens.

Choose the pattern before writing JSX:

| Need | Pattern |
|---|---|
| Many records grouped by a status/stage, moved between groups | **Board** (§1) |
| One record with a body, properties, and history | **Record screen** (§2), opened as its own pane (§3) |
| Comments, replies, and system events on a record | **Activity timeline** (§4) |
| Creating a record | **Creation modal** (§5) — the only modal in the flow |
| Showing a record the agent touched in chat | **Chat card** (§6) |
| Worker settings | **Settings form** (§7) |
| Multi-user consistency | **Live updates** (§8) |

A detail view is never a modal and never a drawer: it is a screen.

## 1. Board (lanes)

Structure: `PageShell` → `PageHeader` (title, `N records · <prefix>` description,
one primary action such as **New ticket**) → `PageMain` → a slim toolbar
(filter `Input`, optional count hint) → a horizontally scrolling row of lanes.

- **Lane** = one surface step: `background: var(--color-surface)`, radius
  `var(--radius-md)`, `flex: 1 0 220px; min-width: 220px; max-width: 360px`,
  column flex with `min-height: 0` and the body `overflow-y: auto`. Never frame
  a lane with a border; the fill is the region.
- **Lane header** (36 px): one 16 px status glyph, 12.5 px/600 sans label,
  tabular mono count in `--color-ink-ghost`, and an `IconButton` “New in
  <lane>” that is `opacity: 0` until `:hover`/`:focus-within` (always visible
  under `@media (hover: none)`).
- **Card** = one step above the lane: `background: var(--color-panel-raised)`
  with a hairline ring `box-shadow: 0 0 0 1px var(--color-edge)`; hover swaps
  the ring for `var(--shadow-raised)`; `:focus-visible` uses `0 0 0 2px
  var(--color-rule-focus)`. Padding 9–11 px, gap 8 px. Line one: 13 px/500
  title. Line two: mono key, a `Chip` only for notable levels (e.g. high /
  urgent), a spacer, and a 20 px initials avatar. Show every fact on the record
  screen instead of turning the card into badge soup.
- **Card semantics**: `<article role="button" tabIndex={0} draggable
  aria-label="KEY: title">`; Enter and Space open it.
- **Empty lane**: a quiet full-width button “No tickets — add one” in
  `--color-ink-ghost` (hover `--color-surface-hover`). Empty board: `EmptyState`
  with the primary action.
- **Loading**: skeleton lanes with `Skeleton` cards so the silhouette is stable.
  **Error**: `StatusPanel variant="alert"` plus a `Button variant="pill"` retry.

### Drag and drop

Use native HTML5 drag and drop; no library.

- `onDragStart` on the card: `effectAllowed = 'move'`, `setData('text/plain',
  key)`, measure the card height, and set the drag state in a **timeout** so the
  browser captures the drag image before the card dims.
- The lane handles `onDragOver` (`preventDefault`, compute the insertion index
  from card midpoints, skipping the dragged card), `onDragLeave` (ignore when
  `relatedTarget` is inside the lane), and `onDrop`.
- **Never remove the dragged card from the DOM.** Keep it in place with
  `data-dragging` (`opacity: 0.35`); a card removed from the list disappears
  for good if `dragend` never arrives. Also clear the drag state from
  document-level `dragend`/`drop` listeners.
- Show the landing position as a **placeholder slot**, not a line: an empty
  block the height of the dragged card with `background:
  var(--color-surface-selected)` and the card radius. Tint the target lane
  `--color-surface-hover`.
- Apply the move **optimistically** with the same ordering rule as the server
  (midpoint between neighbours; append when dropped last), call the move
  function with `{ id, status, index }`, and refetch only on error. Reordering
  inside a lane should not create timeline noise; a lane change should.
- Headless test harnesses do not fire HTML5 drag events from synthetic pointer
  moves: verify by dispatching `DragEvent`s (`dragstart`, `dragover`, `drop`,
  `dragend`) with a `DataTransfer` in `browser::evaluate`.

### Narrow panes

Below the width where two lanes fit (~640 px of pane), show **one lane at a
time** behind `Tabs` → `TabsList variant="line"` → `TabsTrigger icon={false}`
(five tabs in a phone-sized pane is a documented space constraint) with a mono
count after each label; drop the lane header, let the lane fill the width, and
lift cards to 44 px. Remember the active lane per `paneId`.

## 2. Record screen

Structure: `PageShell` → `PageHeader` (title = record title, description =
mono key, actions = optional Back when rendered inline, “Reference in chat”
(`host.chat?.compose`), Delete as an `IconButton` that warms to
`--color-alert` on hover) → `PageMain` with `overflow: auto` → a centered
column (`max-width: 1080px`, padding 18–24 px).

Use a **named-area grid** so the narrow order is a reflow, not a rewrite:

```css
[data-iii-ui="<worker>"] .<worker>-record__grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 268px;
  grid-template-areas: 'head props' 'body props' 'activity props';
  column-gap: 32px; row-gap: 26px; align-items: start;
}
[data-iii-ui="<worker>"] .<worker>-record[data-narrow] .<worker>-record__grid {
  grid-template-columns: minmax(0, 1fr);
  grid-template-areas: 'head' 'props' 'body' 'activity';
}
```

- **Masthead** (`head`): mono key, status `Badge`, priority `Chip`, then the
  18 px/600 title (`letter-spacing: -0.01em`). Rename inline: a pencil
  `IconButton` revealed on hover swaps the heading for an `Input`; Enter or blur
  saves, Escape cancels.
- **Properties rail** (`props`): a `--color-surface` block of label/value rows
  (`grid-template-columns: 76px minmax(0, 1fr)`, 34 px tall, 11.5 px/500 faint
  labels). Controls are inline: `Select appearance="inline"` for finite
  choices, `Selector` with `onCreate`/`allowEmpty` for people, `<time>` with
  relative text and the absolute value on `title`, the UUID in 11 px mono
  ghost. Sticky on wide panes; a two-column grid on narrow ones.
- **Body** (`body`): a section with a 13 px/600 heading row and an
  **Edit** ghost button; read mode renders `Markdown`, edit mode renders
  `CodeEditor language="markdown"` with Save/Cancel in the heading row and
  ⌘↵ to save. Report a dirty draft through `PageRenderProps.setDirty`.
- **Sections** separate by spacing (24–26 px) and heading weight, never by a
  rule.
- **Edits are per field and optimistic**: call the update function on change,
  replace local state with the response, and on failure refetch and show a
  dismissible `StatusPanel variant="alert"` at the top of the screen. Do not
  ship a page-wide “Save changes” form for a record.
- **Delete** goes through `ConfirmDialog` (cancel owns focus), never
  `window.confirm`. After a soft delete keep the screen open with a `StatusPanel
  variant="warn"` (“This ticket was deleted … its file stays on disk”) and a
  **Restore** pill; disable editing while deleted.
- **States**: `Skeleton` blocks in the same grid while loading; `EmptyState`
  when no record is selected (“Open one from the board” with an action that
  opens the board page); `StatusPanel` + retry on load errors.

## 3. Opening a record as its own pane

Register **two pages**: the collection (`<worker>-board`) and the record
(`<worker>-ticket`). From a card, a chat card, or a palette row:

```ts
host.panels?.open({ pageId: '<worker>-ticket', context: { type: 'ticket', id } })
```

The Console places the record page beside the caller in the same workspace
tab, or reuses an open instance, and delivers `PageRenderProps.panelContext`
(`{ id, pageId, context }`). In the record page:

- derive the record id from `panelContext.context` and re-run the effect on
  `panelContext.id` so repeated clicks re-navigate;
- persist the last id in `localStorage` under `<worker>:ticket:<paneId ??
  tabId>` so a reload keeps the pane meaningful;
- render the “nothing selected” `EmptyState` when there is no id.

When `host.panels` is absent (older console), fall back to rendering the
record screen inside the collection pane with a Back action. Keep the record
screen a shared component so both routes look identical.

## 4. Activity timeline and comments

One chronological list (`<ol>`) mixing system events and comments:

- **Event row**: `grid-template-columns: 16px minmax(0,1fr) auto`; a 16 px glyph
  in `--color-ink-ghost`, then `<strong>actor</strong> verb phrase` in
  12.5 px faint sans, then a mono 11 px relative time with the absolute time on
  `title`. Write the verb phrase from field changes with **human labels**
  (“moved this from To do to Done”, “assigned this to Ana”, “edited the
  description”) — never raw enum values.
- **Comment**: a `--color-surface` card with a header row (20 px initials
  avatar, 12.5 px/500 author, mono relative time, a hover-revealed **Reply**
  ghost button) and a `Markdown` body in sans.
- **Replies** nest under their parent in a `--color-panel-raised` card with a
  hairline ring, indented 14 px. Keep threads one level deep; replying to a
  reply attaches to the root.
- **One composer** at the bottom of the timeline on a raised surface: a
  scoped `<textarea>` styled with the input recipe (surface fill, transparent
  border, focus `--color-rule-focus` + 3 px `--color-accent-muted` ring), a
  hint “Markdown supported · ⌘↵ to send”, and one primary button whose label
  flips between **Comment** and **Reply**. When replying, show a context line
  “Replying to <author> · Cancel” above the field; Escape cancels. Do not put a
  reply form under every comment.
- Attribute authorship honestly: the backend defaults `author`/`actor` to a
  neutral value (e.g. `user`) and agents pass `agent`; the UI never hard-codes a
  fake person.

## 5. Creation modal

Creation is the one place a modal fits. `Dialog` → `DialogContent` (width
`min(600px, calc(100vw - 32px))`) → `DialogTitle` + one-line
`DialogDescription` → a form: title `Input` (autofocus, required), description
`<textarea>` (input recipe, 4 rows), then a three-up grid (container query to
one column under ~460 px) of `Select` status, `Select` priority and
`Selector` assignee (`onCreate`, `allowEmpty`, `emptyLabel="Unassigned"`).
Footer: ghost **Cancel**, primary **Create** disabled until the title is
non-empty; Enter submits. Preselect the lane the user clicked. On success:
close the modal, then open the new record as its own pane (§3).

## 6. Chat card for agent calls

Register one `FunctionTriggerRenderer` that claims the worker's record
functions. Unwrap the harness envelope first (see console-injectable-ui.md,
`host.functionTriggers`): the record lives at `output.details`.

- `metadata: { display: true }` so the card stays visible in collapsed call
  groups; `tryRenderDisplay` returns the compact card, `tryRender` the full one
  (with `Markdown` body).
- Card anatomy on `Card interactive`: head row = optional verb (“Created”,
  “Moved to Done”), mono key, status `Badge`, priority `Chip`; then a
  13.5 px/600 title; then a two-line clamped excerpt of the description; then a
  faint footer with assignee avatar + name, comment count, relative “updated”,
  and “Open →” on the right. Whole card is `role="button"` when `host.panels`
  exists and opens the record pane (§3).
- List/board responses render a `TableViewport` → `TableFrame` → `Table
  density="compact"` (key, title, status, priority, assignee), interactive rows
  opening the record, capped at ~25 rows with a faint count footer.
- Return `null` for error envelopes and unexpected shapes so the host's raw view
  takes over.

## 7. Settings form

`SettingsSection` (title + one-sentence description) → `SettingsList` →
`SettingsField` per value. For a path that may be an `${ENV:default}` template,
render `RawValueInput kind="environment"` when the string contains `${`, else
`Input`; show the worker-resolved absolute path as the field `meta` (fetch it
from a small `<worker>::config::info` function). Constrain identifiers in the
`onChange` (e.g. upper-case, `[A-Z0-9]`, length) rather than after save. Add a
read-only `SettingsSection` with `SettingsRow`s for live facts from the worker
(record count, next key). Preserve unknown keys: `onChange({ ...record, key:
next })`. Set `configurationId` on every page registration so the Console owns
the settings action; never render a Configure button or mount
`WorkerConfigurationDialog`.

## 8. Live updates

- The worker owns a trigger type `<worker>:change` (see index.md → Live
  updates) and fires an event for every mutation carrying the **whole record**
  (`{ type, event, ticket_id, ticket, comment?, activity? }`) plus a
  `store.reloaded` event when the backing store changes. The type also supports
  trigger metadata — a `metadata` field in its config — and forwards it on every
  `iii.trigger`, so a binding's handler receives it as its second argument.
- The UI keeps **one binding per tab** shared by every mounted page: a module
  hub with a listener set that registers `host.iii.on(fn, …)` and the
  `registerTrigger` with `function_id: \`${fn}::${host.iii.browserId}\`` on
  the first subscriber and tears both down on the last.
- Collections **upsert from the payload** (remove when `deleted_at` is set) and
  refetch only on `store.reloaded` or events without a payload. Record screens
  merge the record and append unseen comments/activity by id.
- Guard stale responses with a request counter and never overwrite a dirty
  draft; drafts live in separate state from the fetched record.
