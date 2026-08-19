# Console UI conformance inventory

This inventory keeps native Console and runtime-injected worker UI on one
shared visual and interaction contract. Update it whenever a repeated UI
pattern is promoted to `@iii-dev/console-ui`, a worker keeps a local control,
or the selection/motion rules change.

## Shared contract

- Structure: `List`/`ListItem`, `Card`, `Panel`, `Chip`, `IconButton`, the
  semantic `Table` family, and equivalent stable `uiClasses` recipes.
- Choice: shared line `Tabs` or `SegmentedControl variant="tabs"` for peer
  content views, `SegmentedControl variant="radio"` for a persisted exclusive
  choice, `Select` for small finite lists, and `Selector` for searchable
  single choice. Line tabs use a neutral underline, 600 weight, natural case,
  and semantic icons by default.
- Overlays: shared `Tooltip`, `Dialog`, `DropdownMenu`, `Select`, `Selector`,
  and `BottomSheet` preserve injected worker scope through their portals.
- Selection: neutral `surface-selected` + `ink`, with an optional neutral
  `edge`. Accent is reserved for primary actions, form focus, live activity,
  and semantic domain data.
- Motion: public duration/easing tokens and motion recipes; no transition on
  streaming, rapidly updating, dragged, resized, or pointer-following values;
  reduced motion resolves immediately.
- Typography: interface chrome is sans and authored in natural sentence/title
  case. Mono is reserved for machine-readable identifiers, paths, values,
  payloads, code, and tabular data; no panel-wide mono or CSS case transforms.
- Icons: application glyphs use a 16 px baseline. Icon-only actions use
  `IconButton` so the accessible label and shared tooltip remain present; no
  icon usage, component default, or root SVG is below 16 px.
- Tables: `TableViewport`/`TableFrame` owns responsive overflow; semantic table
  parts use natural-case sans headers and horizontal dividers without an outer
  card or border. Comfortable density is the page default, compact density is
  for chat, and mono is applied only to technical cell values.

## Injectable worker sweep

All 22 checked-in injectable UI packages were inspected. “Domain adapter”
means a local component still adds information architecture or semantics; it
must compose shared controls/tokens and is not permission to fork base hover,
selection, tooltip, or selector behavior.

| Worker UI | Shared/conformance result | Retained domain surface |
|---|---|---|
| `browser` | Shared segmented controls; neutral rail/config selection; shared motion tokens | Browser feed, element references, and metadata chips |
| `canvas` | Existing shared controls and token styling audited | Infinite canvas gestures and graph semantics |
| `code-runner` | Shared tooltip and terminal contracts audited | Execution-result composition |
| `computer` | Shared finite selects; neutral session rail; shared motion tokens | Remote-session viewport and controls |
| `console` | Trigger/function filters and rows use public list/chip recipes; neutral selected names and edges | Key/value catalog chips and trigger metadata |
| `cron` | Shared page chrome, line tabs, segmented status filter, table family, list fallback below 760 px, field recipe on every form, shared selector for the function catalog | Cron cadence and next-run labels, schedule status semantics |
| `context-manager` | Minimal shared renderer audited; no selectable navigation | Context accounting payload |
| `database` | Shared line tabs with default icons, icon-only header actions, selects, and tooltips; neutral tree/ERD selection; shared motion tokens | Data grid, query plan, ERD, health metrics, multi-filter chips |
| `editor` | Neutral row/tab/mode selection; shared motion tokens | Editor tab strip and Monaco workspace |
| `eval` | Shared selects/tabs; neutral history/session/run selection; shared motion tokens | Session comparison is intentionally multi-select |
| `github` | Shared line tabs; neutral graph/list selection; shared motion tokens | Commit graph and repository status semantics |
| `harness` | Existing shared controls and token styling audited | Harness run/approval payloads |
| `iii-directory` | Shared line tabs, tooltips, cards, and chips; neutral navigation selection; shared motion tokens | Registry/document editing workflows |
| `llm-router` | Shared finite selects and configuration controls audited | Provider/model configuration semantics |
| `memory` | Local mode toggle removed for shared line tabs; neutral nav/tag selection; shared motion tokens | Memory graph and recall rules |
| `pdf` | Shared tooltip; shared motion tokens | Page rendering and document navigation |
| `provider-openai-codex` | Shared provider-form controls audited; no selectable list shell | OAuth/device authentication flow |
| `sandbox-code-runner` | Shared terminal/tooltip contracts and motion tokens audited | Sandbox lifecycle, file tree, and execution streams |
| `shell` | Local tooltip geometry replaced by a shared-tooltip facade; neutral editor/terminal tabs; shared motion tokens | Terminal, filesystem, and job result adapters |
| `state` | Neutral hierarchy navigation; shared motion tokens | Progressive scope/key/value browser |
| `storage` | Neutral object/config navigation; shared motion tokens | Bucket/object browser |
| `web` | Minimal shared renderer audited; no selectable navigation | HTTP response payload |
| `worktree` | Neutral graph node/edge selection; shared motion tokens | Worktree ownership and graph semantics |

## Native Console sweep

| Surface | Result |
|---|---|
| Workspace tabs and mobile menu | Neutral selected fill/ink; shared control motion |
| Trigger/function catalogs | Public list/chip recipes, shared icon tabs, and neutral selected row/title/edge |
| Schema and chat tables | Shared responsive table parts; natural-case sans headers, horizontal row dividers, selective technical mono |
| Traces, waterfall, and group-by | Neutral selected trace treatment; group-by uses shared `Selector` |
| Chat system-prompt and sheet navigation | Neutral selection and shared overlay behavior |
| Working-directory picker | Neutral current-directory row; hierarchical navigation retained locally |
| Empty-pane page launcher | Public list/panel recipes; persistent always-open command palette retained locally |
| Dialogs, menus, selects, selectors, tooltips, sheets | Shared portal scope and motion vocabulary |
| Streaming context usage | High-frequency width updates are immediate |

## Deliberate local-control exceptions

- `DirectoryPicker` and model/provider navigation use hierarchical drill-in
  and mobile sheet flows rather than a flat searchable selector.
- `SessionAddonsPicker` and evaluation comparison own multi-selection.
- `ReviewScopePicker` owns hierarchical submenu selection.
- `EmptyPane` is a persistent, always-open command palette; it uses shared
  list/panel recipes without adopting popover lifecycle.
- `cron` keeps a local multi-line input (`TextArea`) painted to match the
  shared `Input`, because the library has no multi-line control yet; `eval`
  and `iii-directory` carry the same local copy.
- Graphs, charts, editors, terminals, file trees, ERDs, and canvas surfaces
  may use domain colors and direct-manipulation behavior. Their surrounding
  navigation and selection still follow the shared contract.

## Manual QA matrix

- Light and dark themes.
- 320–430 px phone, narrow split pane, wide pane, and 200% zoom.
- Touch, pointer, keyboard-only navigation, visible focus, names/roles,
  selected state, and live-region output.
- Loading, empty, unavailable, validation error, runtime error, success,
  reconnect, long names/payloads, and offline states.
- Reduced motion plus streaming, log, trace, drag, resize, and pointer-driven
  updates.
- No blue/orange selected names, tabs, chips, card fills, borders, outlines,
  or rails in either theme.
- Content tabs use the shared line treatment with 16 px icons and natural-case
  sans labels; global workspace tabs use weight 500.
- No application icons below 16 px, panel-wide mono typography, or CSS case
  transforms on human-facing controls.
- Shared overlays stay styled inside injected workers; custom portals carry
  the worker's `data-iii-ui` scope.

Automated type, token/class, selection, icon-size, typography, unit,
worker-build, production-build, and Storybook checks complement this matrix.
They do not replace a manual pass in the running Console.
