---
name: design-system
description: The iii Schematic design system — tokens, type, the numbers table, shared components, UX patterns, do/don't.
---

# iii Schematic — design system

The console reads as an engineering document, not a SaaS dashboard: hierarchy comes from
layered surfaces (one background step per region), the chrome draws no lines, every corner is
the same 6px, and a single rationed accent marks live state and primary actions. Sans for
humans, mono for machines. The code is the spec: tokens in `ade/web/src/index.css`, public CSS
recipes in `ade/web/src/styles/ui-recipes.css`, components in `ade/web/src/components/ui`
exported through `packages/console-ui` (`index.d.ts`, `component-names.mjs`, `ui-classes.mjs`,
`token-names.mjs`, `hooks.d.mts`, `format.d.mts`). This file is the only canonical text;
`ade/web/DESIGN.md` is a pointer to it. Workspace behaviour (panes, tabs, chords, palette,
annotations) is documented in `ade/web/README.md`, not here.

## Principles

- **Surfaces, not borders.** A new region is a one-step background change, never an outline or
  divider. Three strokes exist, each with one job: `rule-focus` (focus), `edge` (the hairline
  frame of floating workspace panels and the `PageBody` gap), and an optional neutral `edge`
  rail on a selected row or card. `rule`, `rule-2`, `rule-strong` are transparent legacy names.
- **One radius.** Every Tailwind radius step resolves to 6px; only `none` and `full` differ.
- **Accent rationing.** `accent` (burnt orange on cream, electric blue on dark) is for focused
  controls, live/running state, primary actions and semantic data. Never for selection, body
  text, large fills or decoration.
- **Neutral selection.** Selected = `surface-selected` fill + `ink` text (+ optional `edge`
  rail), identical in both themes. Status arrives through `ok`/`warn`/`alert` on a dot, a mark
  or a `-muted` tint, never a solid background or a stripe.
- **Sans for humans, mono for machines.** Inter for copy and controls; Geist Mono for IDs,
  timestamps, metrics, function names; the code font only on editor, terminal, `pre`/`code`.
- **Motion communicates, never decorates.** Shared duration/ease tokens only; streaming, logs,
  drag and resize update instantly; `prefers-reduced-motion` zeroes every token.
- **Density is deliberate.** Panels, traces, code and forms share a surface without a
  hierarchy contest; the trace timeline is the most colourful area and the only one that draws
  data lines (alpha-ink fills such as `bg-ink/15`).

## Tokens

Use the Tailwind utility (`bg-surface`, `text-ink-faint`) inside the console; injected worker
CSS uses `var(--color-*)`. Public names are the ones in `packages/console-ui/token-names.mjs`;
everything else below is console-only and may change.

### Surface ramp (light → dark)

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `bg` | `#f2f0ed` | `#0a0a0a` | Application canvas, the deepest layer |
| `sidebar` | `#f2f2f2` | `#0e0e0e` | Navigation columns (`PageSidebar`) |
| `panel` | `lab(98.26% 0 0)` | `#111111` | Main columns (`PageShell`, `PageMain`) |
| `panel-raised` | `#f7f5f2` | `#171717` | Headers, cards, popovers, composer, tool cards |
| `paper-2` | `#ebe8e3` | `#171717` | Legacy alias of `panel-raised`; do not add uses |
| `surface` | `rgba(20,16,8,.055)` | `rgba(255,255,255,.055)` | Inputs, controls, chips, secondary cards |
| `surface-hover` | `rgba(20,16,8,.085)` | `rgba(255,255,255,.085)` | Hover on rows, items, ghost controls |
| `surface-selected` | `rgba(20,16,8,.12)` | `rgba(255,255,255,.12)` | Selected row, card, tab, chip, segment |
| `surface-active` | `rgba(20,16,8,.12)` | `rgba(255,255,255,.12)` | Pressed / strong active |
| `card-highlight` | `#dbdbdb63` | `#0d0d0e63` | Borderless inset inside a card; never a state |

The `surface*` fills are alpha, so one step reads the same over any base layer.

### Ink, strokes, accent, status

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `ink` | `#0a0a0a` | `#ededed` | Primary text, primary buttons, wordmark |
| `ink-faint` | `#6b6865` | `#a6a6a6` | Secondary copy, captions, inactive navigation |
| `ink-ghost` | `#a3a09c` | `#6f6f6f` | Placeholders, timestamps, line numbers |
| `ink-disabled` | `#b8b4ae` | `#4d4d4d` | Disabled labels (paired with `opacity-40`) |
| `muted-foreground` | `#6b6865` | `#b8b8b8` | Legacy alias of `ink-faint` (dark one step brighter) |
| `trigger-running` | `#57534f` | `#a6a6a6` | In-flight function description shimmer base |
| `rule`, `rule-2`, `rule-strong` | transparent | transparent | Legacy; `border-rule*` is an inert 1px |
| `rule-focus` | `rgba(184,66,15,.6)` | `rgba(40,168,247,.7)` | The focus stroke on inputs and controls |
| `edge` | `#14100829` | `rgba(255,255,255,.07)` | Panel frame, `PageBody` gap, selected rail |
| `accent` / `accent-fg` | `#b8420f` / `#f2f0ed` | `#28a8f7` / `#070909` | Live state, focus ring, primary action |
| `accent-hover` | `#a53a0c` | `#46b6fa` | Hover on accent fills |
| `accent-muted` / `accent-border` | 10% / 35% alpha | 12% / 35% alpha | Accent tint / legacy accent stroke |
| `alert` / `alert-muted` | `#ff0026` / 8% | `#f05d68` / 12% | Errors, failed calls |
| `warn` / `warn-muted` | `#a87a00` / 12% | `#f5a524` / 12% | Warnings, pending approval |
| `ok` / `ok-muted` | `#356f3d` / 12% | `#36c98f` / 12% | Success, completed calls, diff additions |
| `workdir` / `workdir-muted` | `#0ea5e9` / 12% | same / 14% | Console-only: the chat activity folder mark |
| `ring` | 5% `ink` mix | same | Tailwind's default `ring` colour |

### Glyph tones

`glyph-blue #2563eb`, `glyph-purple #7c3aed`, `glyph-teal #0d9488`, `glyph-green #15803d`,
`glyph-amber #b7791f`, `glyph-rose #e11d48` (dark: `#3b82f6 #8b5cf6 #14b8a6 #22c55e #eab308
#f43f5e`). Identity tints for one 16px glyph beside a label — an agent profile in the
conversation tree, a folder or scope mark in a worker's tree — applied through `data-color` on
`uiClasses.treeItemIcon`. Each holds at least 3:1 against `sidebar`. A tone is never a fill, a
border, text or a selection/status state; `neutral` means the glyph keeps `ink-ghost`.

### Fonts — three roles

| Token | Stack | Role |
| --- | --- | --- |
| `--font-sans` | **Inter**, ui-sans-serif, system-ui | Human copy and controls (400/500/600 loaded) |
| `--font-mono` | Geist Mono, Chivo Mono, ui-monospace | Machine values in chrome: IDs, times, eyebrows, keys |
| `--font-code` | Monaco, SF Mono, Menlo, Consolas | Editor, terminal, `pre`, `code` — not the chrome mono |
| `--font-geist-mono` | Geist Mono, ui-monospace | Alias used by function-trigger chrome |

Ligatures are off on every mono/code surface (`liga clig calt dlig 0`). Do not add a family.

### Radii, shadows, motion, spacing

| Token | Value |
| --- | --- |
| `radius-xs` … `radius-xl` | 6px, all of them; `radius-none` 0; `radius-full` 9999px |
| `shadow-raised` (light) | `0 1px 2px #1410081a, 0 4px 12px #1410081a` |
| `shadow-raised` (dark) | `0 1px 0 rgba(255,255,255,.025) inset, 0 8px 24px rgba(0,0,0,.18)` |
| `shadow-floating` (light) | `0 2px 4px #1410081a, 0 10px 24px #1410081a` |
| `shadow-floating` (dark) | `0 1px 0 rgba(255,255,255,.03) inset, 0 12px 32px rgba(0,0,0,.28)` |
| `shadow-lift` | inset 1px highlight + inset 1px ring + 1px outer edge + two tight drops (composer) |
| `shadow-keycap` | `lift` upside down — lit edge at the bottom, drops upward (`Kbd` only) |
| `motion-duration-instant/fast/control/panel` | 0 / 120 / 160 / 220 ms |
| `motion-ease-standard` | `cubic-bezier(0.2, 0, 0, 1)` — state changes |
| `motion-ease-enter` / `ease-glide` | `cubic-bezier(0.16, 1, 0.3, 1)` — mounting |
| `motion-ease-exit` | `cubic-bezier(0.4, 0, 1, 1)` — dismissal |
| `spacing-gutter` / `section-x` / `section-y` | 24 / 36 / 80 px |
| `spacing-sheet-max` / `content-max` | 1200 / 1216 px |

Four shadows, each a complete theme-aware value; the `--iii-ui-lift-*` ingredients are
console-only. Under `prefers-reduced-motion` the three non-zero durations become 0ms and every
animation runs once at 0.01ms. Console-only transitions.dev scale: `--duration-stagger 40`,
`micro 80`, `quick 150`, `fast 250`, `medium 350`, `slow 400`, `very-slow 500` ms,
`--ease-smooth-out`, `--distance-*`, `--scale-*`, `--blur-*` — used by dropdown, sheet and
picker-page motion. Note `--duration-fast` (250ms) is not `--motion-duration-fast` (120ms).

Dark theme is the same token set flipped under `html[data-theme="dark"]`: a neutral gray ramp
from black, white-alpha component fills, blue accent, and heavier shadows because there the
shadow is the only depth cue. `index.html` picks the theme before paint from `localStorage`
`iii-theme`, else the OS `prefers-color-scheme`; `useTheme` (`use-theme.ts`) then owns
`html[data-theme]`, `color-scheme`, the `theme-color` meta and the stored value. Once a user
has chosen, the console does not follow later OS changes.

## Typography

Two families plus the code font (above). The scale is whatever the shared components render:

| Role | Size / weight | Where |
| --- | --- | --- |
| Page title | 14px / 500 sans, ink | `PageHeader` `title`; `description` 12px ink-ghost |
| Cell title | 16px / 600 sans, `-0.01em` | `EmptyState` heading |
| Card / panel header | 13px / 600 sans on `surface`, 40px tall | `CardHeader`, `PanelHeader` |
| Settings section title | 14px / 600 desktop, 16px phone | `SettingsSection` |
| Body | 13px sans (`body` element: 14px/20px) | list titles, tree rows (500), tabs (600), buttons |
| Description | 12px ink-faint | list/settings descriptions, `StatusPanel` detail, tooltip |
| Eyebrow | 11px / 500 mono, uppercase, `0.06em`; `lg` `0.14em` | `Eyebrow`, `.iii-ui-eyebrow` |
| Metadata | 11px sans, `tabular-nums` | `StatusBar`, tree meta, field description, `Kbd` (mono) |
| Floor | 11px | `lint-worker-ui` warns below it |
| Micro | 10px mono | Trace timeline labels only — data visualisation, never chrome |

Every number, timestamp and KPI sets `tabular-nums`. Case: copy keeps its authored case. The
only sanctioned CSS case transform is the eyebrow (`Eyebrow`, `eyebrowClassName`,
`.iii-ui-eyebrow`); no `uppercase`/`lowercase`/`capitalize` anywhere else, on any element.

## Numbers

The single source of truth — other skills link here and never restate these.

| Number | Value | Source |
| --- | --- | --- |
| Body text | 13px desktop; inputs 16px on phones | `Input.tsx`, `ui-recipes.css` |
| Page title / description | 14px / 12px | `PageChrome.tsx` |
| Card header | 13px / 600 | `.iii-ui-card__header` |
| Metadata / eyebrow / floor | 11px | `Toolbar.tsx`, `Eyebrow.tsx`, `lint-worker-ui.mjs` |
| Icon | 16px (`size-4`, `.iii-ui-icon`) | `ui-recipes.css`, lint rule `icon-size` |
| Radius | 6px | `--radius-*` |
| Desktop rows | tree row 28, status bar 28, toolbar 36, page header 44 | `ui-recipes.css`, `PageChrome.tsx` |
| Desktop controls | icon button 30, button sm/md/lg 32/36/44, input 36 | `Button.tsx`, `Input.tsx` |
| Narrow pane | tree rows 44, tree caret/action 40 (`data-narrow`) | `.iii-ui-tree[data-narrow]` |
| Narrow container | settings controls and actions 48 under 30rem | `@container (max-width: 30rem)` |
| Phone (< 640px) rows | list rows 48, segmented items 44, settings rows 56 | `ui-recipes.css` |
| Phone icon actions | header close and gear 48, sheet close 48, switch hit 48 | `PageChrome.tsx`, `BottomSheet.tsx` |
| List row | 44px min everywhere, 48 on phones | `.iii-ui-list-item` |
| Chrome breakpoint | 640px viewport (`sm:`), phone vs desktop presentation only | `use-media-query.ts` |
| Pane steps | `@2xl` 672px, `@3xl` 768px, `@5xl` 1024px container queries | `PageShell` is `@container` |
| Narrow-pane threshold | 720px default (`useContainerNarrow`); chat pane uses 560 | `hooks.d.mts`, `ChatPanel.tsx` |
| Settings row stacking | under 30rem (480px) container | `.iii-ui-settings-row` |
| Table text | 14px, 13px at ≥ 40rem container; cells 12px, compact 8px | `.iii-ui-table` |
| Panel gutter | 6px between floating workspace panels | `workspace-panel-divider-*` |
| Card padding | body 12px; `EmptyState` cell 20px; header 10px/12px | `.iii-ui-card__body`, `Cell.tsx` |
| Tooltip | bottom by default, offset 6px, 12px text | `Tooltip.tsx` |

## Shared components

Everything below is exported by `@iii-dev/console-ui` (`component-names.mjs`); props are the
ones in `index.d.ts`. State is expressed with `data-*` attributes (`data-selected`,
`data-tone`, `data-narrow`, `data-density`), never with extra classes.

### Page chrome

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `PageShell` | Pane root column, `bg-panel`, `@container` | Do make it every page's outermost element |
| `PageHeader` | `icon` 16px, `title`, `description`, `actions`, `onClose` | Do pass `onRequestClose`; no custom top bar |
| `PageBody` | Row under the header; `side`; 1px `bg-edge` gap | Don't add borders between sidebar and main |
| `PageSidebar` | `bg-sidebar`; `collapsible`, `resizable`, `storageKey`, `narrowBelow` | Do give it `label`; no own resize hook |
| `PageMain` | The primary column, `bg-panel`, scrolls inside | Do put toolbars and status bars inside it |

`PageSidebar` also takes `width`/`minWidth`/`maxWidth`, `narrow` and `narrowMode`
(`inline` full-width drill-in or `drawer` overlay).

### Structure

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `Card` / `CardHeader` / `CardBody` | `panel-raised` + `edge` + `shadow-raised`; `selected`, `interactive` | No dividers |
| `CardHighlight` | Borderless neutral inset (`card-highlight`) | Never for hover, selection or status |
| `CollapsibleCard` + `Trigger` + `Content` | Grid-track height motion; content stays mounted | No button inside the trigger |
| `Panel` / `PanelHeader` / `PanelBody` | Same recipe as Card without the shadow | Do use for in-flow regions |
| `Toolbar` | 36px raised strip, `end` slot, `role="toolbar"` | Do name it with `aria-label` |
| `StatusBar` | 28px quiet strip, 11px faint tabular text, `end` slot | Counts and paths here, not the header |

### Lists and trees

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `List` / `ListGroup` / `ListGroupLabel` | 4px gap; arrows and Home/End walk items | Do group with a 12px/600 label |
| `ListItem` | 44px button row: `leading`, `label`, `description`, `trailing`, `selected` | Never accent on selection |
| `uiClasses.tree*` | 28px rows, `--iii-ui-tree-depth`, `treeItemIcon[data-color]`, caret, `meta`, `actions` wrapping every `action` | Do wrap the actions — a hidden one must never hold width. Phones: `data-narrow`; `data-pointer="fine"` drops a mouse-only action |

### Tables

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `TableViewport` → `TableFrame` → `Table` | Scroll viewport (`@container`), frame, `density` comfortable/compact | Wrap it |
| `TableHeader` / `TableBody` / `TableFooter` | Sections; rows separate with an `edge` hairline | Don't zebra-stripe |
| `TableRow` / `TableHead` / `TableCell` / `TableCaption` | `interactive` rows walk with arrows; `selected` | No vertical rules |

### Choice and input

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `Tabs` + `TabsList`/`TabsTrigger`/`TabsContent` | Line variant, 40px, 2px ink underline, `icon` from `value` | Content navigation |
| `SegmentedControl` | `variant` `tabs` (tablist) / `radio` (preference), `iconOnly` | `aria-label` on `radio` |
| `Select` | `options`/`groups`, `appearance`, `sheetTitle` on phones | Short fixed lists |
| `Selector` | Searchable combobox; `query`, `loading`, `error`, `onCreate`; needs `aria-label` | Long or remote lists |
| `Switch` | 36×20 desktop, 44×24 phone, 48px hit area | State only, never an action |
| `Input` | 36px desktop / 48px phone, `surface` fill, `rule-focus` on focus | Don't add a border |
| `SearchField` | Magnifier, clear button, Escape clears and stops | Use it for filtering, not a bare `Input` |
| `RawValueInput` | `Input` that preserves raw values | Machine values |
| `SettingsSection` / `SettingsList` | `title`, `description`, `action`; the list frames rows | Compose forms from these |
| `SettingsRow` | `label`, `description`, `meta`, `control`, `action`, `layout` auto/inline/stacked | No hand-rolled label pairs |
| `SettingsField` | `SettingsRow` owning ids and ARIA via `renderControl`; `controlSize`, `error` | Every form control |
| `SettingsDeck` | Overview ↔ detail: `open`, `overview`, `detail`, `title`, `onBack` | Drill-in settings on narrow panes |

### Actions

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `Button` | `variant` primary/ghost/pill/icon/terminal/wiggle; `size` sm 32 / md 36 / lg 44 / icon 30 | One `primary` per view |
| `IconButton` | `Button` size icon + required `label`, `tooltip` (false hides), `tooltipSide` | Never icon-only without a label |
| `Tooltip` | `label` shorthand, or `TooltipTrigger` + `TooltipContent` (`side`, default bottom) | Don't tooltip a bare ✕ |
| `DropdownMenu` + `Trigger`/`Content`/`Item`/`Label`/`Separator` | Radix menu on `panel-raised`, `shadow-floating` | Destructive last |

### Feedback

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `EmptyState` | `icon`, `title`, `description`, one `action` (ghost sm) | Write a sentence, not a shrug |
| `Skeleton` | `surface` block with `skeleton-pulse` | Match the final layout's shape |
| `StatusPanel` | `variant` info/success/warn/alert; `icon`, `headline`, `detail` on the tint | Errors + retry; never raw text |
| `Badge` | Pill, `variant` default/ok/warn/alert/accent, 12px (16px phone) | One word or a count |
| `Chip` | 24px tag, `tone` neutral/accent/success/warning/danger, `selected` | Not a button |
| `StatusDot` | 6px dot, `tone` accent/alert/warn/ink, `pulse` (`pulse-dot`, the one glow) | Accent = running |
| `LiveRegion` | `announcement` `{ seq, text, urgency }` | Announce async results once |
| `Eyebrow` | Mono caps label, `as`, `size` md/lg | Don't uppercase anything else |
| `Kbd` / `KeyCombo` | Key cap with `shadow-keycap`; `binding` (`Mod+S`) per `platform` | Shortcuts in prose |
| `ErrorBoundary` | Catches render errors of injected UI | Wrap independently failing regions |

### Overlays

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `Dialog` + `Trigger`/`Content`/`Title`/`Description`/`Close` | Radix dialog on `panel-raised`, `shadow-floating` | Always a title |
| `ConfirmDialog` + `useConfirm` | `title`, `description`, `details[]`, `tone` danger; cancel owns focus | Never `window.confirm` |
| `BottomSheet` + `Content`/`Title`/`Description`/`Trigger`/`Close` | Phone sheet, `heading`, in-sheet pages, safe area | Desktop: popover |
| `ImageViewer` / `ImageThumbnailButton` | Full-screen viewer with zoom/pan; thumbnail opens it | Any content image |
| `AnnotationLayer` / `AnnotationList` | Pin/rect/arrow annotations over an image | Screenshot markup |

### Code and terminal

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `CodeEditor` | `value`, `language`, `readOnly`, `fill`, `lineNumbers`, `wordWrap`, `selectionActions` | `fill` in a pane |
| `FileDiff` | Diff of two `FileDiffSide`s | Every file change |
| `Markdown` / `MarkdownPreview` | Rendered markdown (`children` / `markdown`) | No `dangerouslySetInnerHTML` |
| `CodeHighlight` / `JsonHighlight` | Static highlighted block, `wrap` | Read-only snippets |
| `TerminalCommandLine` | Prompt + `command`, optional `copy`, `chips` | Keep it in `--font-code` |
| `TerminalStream` / `AnsiText` | Streamed `text`, `tone` out/err, `ansi`, `clampLines` | Clamp long output |

### Activity and identity

| Component | Anatomy / props | Do / Don't |
| --- | --- | --- |
| `MetaRow` | Label/value strip a function-call card opens with; `items[]` values in mono | Call metadata |
| `ActionLine` | One reported action: `icon` 16px in `tone` accent/warn/ink, body in ink | `→ url`, `ƒ function` |
| `ModelPicker` | `value`, `options`, `thinkingLevel`, reasoning effort, provider setup | No second model menu |
| `DirectoryPicker` | Working-directory picker with browse and validation | Wherever a page needs a folder |
| `Wordmark` | `appearance` default/inset/loading, `tone` auto/ink/inverse | Never coloured |
| `WorkerConfigurationDialog` | Opens a worker's configuration by `configurationId` | Prefer `PageHeader`'s gear |

### Hooks and helpers

| Export | What it does |
| --- | --- |
| `useTheme()` | Public: `'light' \| 'dark'`; the console's `use-theme.ts` also returns a setter |
| `useConfirm()` | `{ confirm, dialog }` — render `dialog` once, `await confirm(options)` |
| `useContainerNarrow({ below })` | `{ ref, narrow }` from a ResizeObserver on the pane (default 720) |
| `usePaneState(key, initial)` | `useState` mirrored to localStorage; key it by `paneId` |
| `useCopyFlash(text, ms)` | `{ state: idle\|copied\|failed, copy }` |
| `useWorkerLive({ iii, triggers, fetch, handlerId })` | Fetch once, refetch on trigger events, poll only while not live |
| `formatRelative`, `formatDuration`, `formatBytes` | `42s`, `2m 05s`, `1.0 MiB` — binary units with a space |
| `errorCode`, `errorMessage`, `copyText` | Readable errors from a rejected `iii.trigger`; clipboard with fallback |

The four hooks are `@iii-dev/console-ui/hooks`; the formatters are `@iii-dev/console-ui/format`
— both bundle into a worker page.

## UX patterns

| Pattern | Do this |
| --- | --- |
| Loading | `Skeleton` in the shape of the final content, in place; never a spinner over a blank pane |
| Empty | `EmptyState` with a title, one sentence and at most one action |
| Error | `StatusPanel variant="alert"` + `errorMessage(err)` + a retry `Button`; never raw error text |
| Selection | `data-selected` / `aria-selected` / `aria-current` → `surface-selected` + `ink`; never accent |
| Toolbars | `Toolbar` above the content, `StatusBar` below; both inside `PageMain` |
| Search / filter | `SearchField` (Escape clears); debounce remote queries in the page |
| Confirmation | `useConfirm` / `ConfirmDialog`; `tone="danger"` for destructive; never `window.confirm` |
| Eyebrows | `Eyebrow` for section labels and key/value keys; no other uppercase |
| Icons | `lucide-react` through the import map, 16px; never inline `<svg>` or text glyphs (`✓`, `×`) |
| Narrow panes | `useContainerNarrow` / `PageSidebar narrowBelow`; `data-narrow` on trees; never viewport queries |
| Live data | `useWorkerLive` with the worker's trigger types; show `live` quietly, never a banner |
| Formatting | The `format` subpath; bytes binary with a space (`3.2 MiB`), durations `1.4s` / `2m 05s` |
| Pane state | `usePaneState` keyed by `PageRenderProps.paneId`; two panes of one page stay separate |
| Keyboard | `PageRenderProps.commands.register([...])` in an effect; no document listeners; no bare keys |
| Motion | `uiClasses.motionControl/motionPanel/motionOverlay`, `uiClasses.spin` (1s), `uiClasses.pulse` |
| Identity | `glyph-*` on one 16px glyph via `treeItemIcon[data-color]`; status stays on `StatusDot`/`Chip` |
| Mono vs sans | Values in `font-mono`, labels in sans; never a whole panel in mono |
| Function-call cards | `MetaRow` first, `ActionLine` per reported action, body in `fcall-chrome` (console) |
| Theme | Read `useTheme()`; never branch on `prefers-color-scheme` yourself |

## Do / Don't

**Do**

- Build every page from `PageShell` → `PageHeader` → `PageBody` → (`PageSidebar`) + `PageMain`.
- Step the background one level to make a region; reach for `edge` only on the panel frame, the
  `PageBody` gap and a selected rail.
- Use Inter for copy and controls, Geist Mono for machine values, the code font for code.
- Keep every number `tabular-nums`; keep authored case; `Eyebrow` is the one uppercase.
- Ration the accent to focus, live/running state, primary actions and semantic data.
- Lay panes out with `@container` and `@2xl:` / `@3xl:` / `@5xl:`; reserve the 640px viewport
  breakpoint for console chrome (phone sheet vs popover, 16px inputs, 48px touch targets).
- Use the four shadows (`raised`, `floating`, `lift`, `keycap`) and the motion tokens; honour
  reduced motion by default (the tokens already do).
- Use Lucide icons at 16px and give every icon-only control a `label`.

**Don't**

- Don't draw dividers, outlines or accent selection strokes; don't add `border-rule*`.
- Don't invent radii, shadows, durations, font sizes below 11px, or a fourth font.
- Don't put a solid status colour behind text; tint with `-muted` and colour the text/dot.
- Don't uppercase, lowercase or capitalise copy outside `Eyebrow`.
- Don't use viewport media queries for pane layout, `window.confirm`, inline SVG, document key
  listeners or bare-key shortcuts.
- Don't use gradients, except `fcall-chrome` (function-call cards) and `thinking-shimmer`
  (streaming text), and no glow except `pulse-dot`.
- Don't set a whole panel in mono, or technical values in sans.
- Don't reach for a Tailwind utility to override a property a recipe already sets:
  `ui-recipes.css` is imported unlayered, so a normal utility in `@layer utilities` loses to it
  at any specificity and silently does nothing. Only an `!important` utility would win, and that
  is not the answer — put the rule in the recipe, keyed by a data attribute.
