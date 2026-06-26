# Spec: resizable conversation side panel

> **IMPLEMENTED (2026-06-26).** `use-sidebar-width.ts` (clamp/persist),
> drag handle + `isResizing` in `ConversationSidebar`, wired from `ChatPanel`.
> `density` dropped from the sidebar (width is now a prop). Clamp unit test +
> full suite (780) green.

## Goal

Let the user drag the **conversation list** (`ConversationSidebar`) to a custom
width and have it persist across reloads. Today the list is a fixed `w-[220px]`
(dock density) with only a collapse/expand toggle.

> Note: the **outer `ChatDock`** is already resizable (`ChatDock.tsx` drag
> handle + `use-chat-dock.ts`). This spec is about the **inner conversation
> list** inside the dock — a separate panel. The dock holds
> `[ConversationSidebar][ChatView]`; we make the first one draggable.

## Scope

- Only the `dock` density matters: `ChatPanel` is rendered solely as
  `density="dock"` in the app (`ChatDock.tsx:122`); `route` density exists only
  in Storybook. So we persist one width and don't branch per-density.
- Drag handle on the **right edge** of the expanded sidebar (mirrors the dock:
  anchored left, drag right = wider). Collapsed state is unchanged (the `w-9`
  icon strip; no handle).

## Reuse — copy, don't invent

The complete, accessible pattern already lives in the dock. Lift it almost
verbatim:

- `ChatDock.tsx:22` `clamp`, `:61-95` mousedown→track `dx`→`onWidthChange`,
  `:97-99` double-click reset, `:124-139` `role="separator"` handle with
  `aria-valuenow/min/max`.
- `use-chat-dock.ts` — load/clamp/persist + viewport re-clamp. The new hook is a
  trimmed copy (no collapse, no global keyboard shortcut — collapse already
  lives in `ChatPanel`).

## Changes

### 1. New hook `web/src/hooks/use-sidebar-width.ts`

Mirror `use-chat-dock.ts`, width-only:

```ts
const WIDTH_KEY = 'iii-chat-sidebar-width'
export const SIDEBAR_DEFAULT_WIDTH = 220   // current dock width
export const SIDEBAR_MIN_WIDTH = 160
export const SIDEBAR_MAX_WIDTH = 420       // ponytail: fixed cap; see clamp note
```

- `loadWidth()` / `persistWidth()` against `localStorage` (best-effort
  try/catch, same as the dock hook).
- `clampWidth(w)` → `Math.max(MIN, Math.min(MAX, w))`.
- Return `{ width, setWidth }`. `setWidth` clamps before set + persist (effect).

`// ponytail: fixed MAX_WIDTH instead of measuring the dock. The sidebar sits
inside the dock (default 440px) next to ChatView (flex-1). A 420 cap leaves
≥~20px for ChatView at the default dock width; if the user wants more list, they
widen the dock too. Upgrade to a measured `dockWidth - CHATVIEW_MIN` clamp only
if that coupling proves annoying.`

### 2. `ConversationSidebar.tsx`

- Accept `width: number`, `onWidthChange: (n: number) => void`.
- Expanded branch: replace `widthClass` (`w-[220px]`/`w-[260px]`) with inline
  `style={{ width }}` on the `<aside>` (keep `shrink-0`).
- Render the drag handle as a sibling right after the `<aside>` (same JSX block
  as `ChatDock.tsx:124-139`): `role="separator"`, `aria-orientation="vertical"`,
  `aria-valuenow/min/max`, `aria-label="resize conversations"`, `tabIndex={0}`,
  `onMouseDown`, `onDoubleClick` reset, `cursor-col-resize` + accent hover.
- Lift the `isResizing` state + the mousemove/mouseup `useEffect` from
  `ChatDock.tsx:47-95` into this component (or a tiny shared
  `useColResize(width, onWidthChange, clamp)` helper if we want to dedupe with
  the dock later — **skip for now**, two call sites don't justify the
  abstraction).
- Collapsed branch: unchanged. No handle when collapsed.

### 3. `ChatPanel.tsx`

- Call `useSidebarWidth()` (it already owns `sidebarCollapsed`).
- Pass `width` / `onWidthChange` down to `ConversationSidebar`.

That's the whole wiring — `ChatPanel` is the natural owner since it already
holds the collapse state for the same panel.

## Tests

- `use-sidebar-width.test.ts` (mirror any existing hook test): clamp below MIN →
  MIN; above MAX → MAX; persist round-trips; bad localStorage → default.
- If `ConversationSidebar` has a story/test, add a "resized" story at a non-default
  width so the handle + inline width render is visually covered.

## Risks / notes

- **Squeezing ChatView**: bounded by `SIDEBAR_MAX_WIDTH`; the parent flex keeps
  ChatView `flex-1`. Worst case at a narrow dock the user drags to MAX — ChatView
  still renders (min content), and the dock itself is resizable. Acceptable.
- **Collapsed → expand restores width**: width state is independent of collapse,
  so expand returns to the last dragged width for free.
- **Drag handle a11y**: keep the dock's `role="separator"` + `aria-value*`
  attributes; that's the accessible contract already shipped for the dock.

## Estimate

~40–60 lines (one hook + handle JSX + 3 prop wires). Half a day with tests.
Independent of the tree-view spec — ship either order.
