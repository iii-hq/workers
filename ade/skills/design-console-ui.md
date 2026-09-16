---
name: design-console-ui
description: Responsive UX and configuration forms for iii Console interfaces — native surfaces and runtime-injected worker pages, function and trigger-activity renderers, worker configuration forms, and provider-owned configuration or authentication flows — across phones, narrow split panes, and desktops. Use for archetype choice, pane-width behavior, phone drill-in, bottom sheets, state integrity, configuration editors, model/provider setup, touch and keyboard accessibility, and the validation matrix. Delivery lives in ade/injectable-ui; visual rules and every number live in ade/design-system.
---

# Design iii Console UI

Create operator interfaces that feel native to the Console, preserve
host-owned state and safety boundaries, and remain fully usable on phones,
narrow split panes, and wide desktops. This skill owns behavior across widths
and configuration forms. How a worker ships UI (slots, build, lint, scoped
CSS, registration) is `ade/injectable-ui`; tokens, typography, component
anatomy and every size, target and breakpoint are `ade/design-system` —
link to `ade/design-system` › Numbers, never restate a number. Inspect the
target implementation and `packages/console-ui/index.d.ts` when integrating;
do not invent components, slots, or props.

## Work in this order

1. Inventory the behavior, data, states, actions, dirty drafts, async work, and existing constraints before changing markup.
2. Choose one archetype and one information architecture for the primary task. Define the wide flow and the narrow flow separately.
3. Decide whether the surface belongs to the Console itself, a full injected page, a function renderer, a trigger-activity renderer, a worker configuration form, a provider configuration form, or a compact chat slot (`ade/injectable-ui` › Slots).
4. Keep authoritative data, validation, persistence, and navigation guards in the host. Let an injected worker own presentation and worker-specific calls.
5. Build from shared primitives and design tokens. Add the least scoped CSS needed for the domain.
6. Exercise loading, empty, unavailable, unconfigured, dirty, saving, saved, error, reconnect, and stale-response states.
7. Verify phone, narrow-pane, and desktop behavior in both themes with touch, mouse, and keyboard before declaring the UI complete.

## Archetypes

Choose one dominant archetype before writing JSX. Mixing them produces a
generic dashboard with too many panels. Derive sidebar counts, thresholds and
controls from the worker's content, never from a reference's numbers.

| Archetype | Use for | Wide shape | Narrow shape |
|---|---|---|---|
| Console catalog | Many searchable objects with rich detail | Grouped list → persistent hero or breadcrumb + identity masthead + tabs; a contextual rail only for genuinely related information | List, then detail, one level at a time |
| Database workbench | Several tools operating on one selected resource | Compact mode switcher, collapsible resource tree, one active work surface, local toolbar and status bar, optional inspector | Tree as a full-width list; one tool at a time; inspector as a sheet |
| Directory editor | Searchable documents with drafts or preview | List → document identity → edit/preview modes; draft state stays mounted, save status beside the work | One mode at a time; edit and preview never side by side |
| State explorer | Deep but compact hierarchy | Progressive columns | One level at a time with a labelled Back |
| Settings flow | One configuration entry, host-owned persistence | Centered contained column; `SettingsDeck` for collections | Same column; deck opens one level |
| Terminal/instrument | One live surface (terminal, viewport, feed) | Toolbar above, status bar below, side rail for sessions | Rail becomes a list; surface fills the pane |

Reference implementations per archetype: `ade/injectable-ui` › Living references.

## Model the Console as responsive workspace software

Do not shrink desktop UI into a phone. Change the interaction model when the
available width changes.

### Use the correct width signal

- The viewport breakpoint belongs to the Console chrome only: phone
  presentation below it (bottom sheet instead of popover, the phone menu,
  phone-sized inputs and targets, safe-area padding), desktop at or above.
  Native code allowlists each remaining viewport utility with a reason
  (`ade/web/src/lib/viewport-breakpoint-conformance.test.ts`).
- Content inside a workspace pane responds to the pane. Every `PageShell` is
  a container: use `@container` in CSS for visual changes, and
  `useContainerNarrow` from `@iii-dev/console-ui/hooks` when React must
  mount a different narrow view (synchronous first measure, resizes
  observed, zero-width hidden panes ignored). A split desktop pane can be
  narrower than a phone viewport.
- Derive a container threshold from the minimum usable content width; the
  hook's default and the viewport breakpoint are in `ade/design-system` ›
  Numbers. Never copy a threshold without checking the target content.

### Change structure on phones and narrow panes

- Replace side-by-side master/detail with a drill-in sequence: list →
  detail, scope → resource → value, or settings → category → choice. Render
  one primary page at a time with a visible, labelled back action; a row
  advances exactly one level. Never flatten parent and child collections
  into one selector or show a collapsed desktop rail first.
- Keep `PageSidebar` in its default inline narrow mode for that sequence; its
  full-width presentation is shared. Do not recreate rails, sheets, width
  overrides, or collapse state in worker CSS/JS. `narrowMode="drawer"` is
  only for secondary navigation over an unchanged, still-mounted `PageMain`.
- Build each level with `List`, optional `ListGroup`/`ListGroupLabel`, and
  `ListItem` (`selected`, `leading`, `label`, `description`, `trailing`)
  instead of copying row CSS; the shared row owns full-width targeting,
  neutral selection, keyboard traversal, focus, and touch height.
- Remove modes that require width: collapse split edit/preview to one mode
  at a time. Use `panelSide` to mirror side navigation in a wide right-hand
  pane, never reading order or a single-pane narrow flow.
- Keep editors mounted when hiding a mode if cursor and scroll continuity
  matter; unmount when state must reset between domain objects.

### Size interaction deliberately

- Primary rows, icon actions, and form controls meet the touch targets in
  `ade/design-system` › Numbers on phones and in narrow split panes even
  when the desktop window is wide; compact desktop controls may shrink to
  the desktop size there. Phone text inputs use the phone input size so the
  browser does not zoom.
- Never hide an essential action behind hover on coarse pointers. Use
  `pointer-fine` only for hover-only disclosure. When a small visual icon
  must stay compact, enlarge its invisible hit area without changing layout.
- Keep focus rings visible, name every icon-only action (`IconButton`), mark
  decorative icons `aria-hidden`, and expose selected state with
  `aria-pressed`, `aria-current`, radio semantics, or a checkmark — not
  color alone.

### Compose phone sheets correctly

`BottomSheet` (`BottomSheetContent`, `BottomSheetTitle`, …) is the shared
phone overlay: modal, inset, rounded raised panel, drag handle, close
target, `dvh`-based maximum height, safe-area padding, scope-preserving
portal. Compose on it; do not build a local sheet.

- Replace competing dropdowns and dialogs with one sheet and an in-place
  navigation stack (`push`, `back`, `reset`); avoid duplicate consecutive
  pages, reset after a successful close, and never open a second portal on
  top of the sheet for a sub-selection.
- Keep selector logic presentation-independent so desktop dropdowns and
  sheet pages share options, selected value, disabled rules, and callbacks.
- Keep dangerous confirmation as another page in the same sheet, or use
  `useConfirm()`; run the same unsaved-change guard for back, close, overlay
  dismissal, and sheet teardown.
- Keep the sheet header fixed; only the content scrolls
  (`overscroll-behavior: contain`). Use grouped rows with a strong label,
  quiet current value, optional icon, and chevron; radio-style rows for
  mutually exclusive choices.
- Avoid autofocus that opens the phone keyboard as soon as a sheet appears;
  keep keyboard-first autofocus in desktop popovers where useful.

### Compose phone workspaces correctly

- Turn multiple panes into full-width horizontal snap pages on phones
  (`snap-x`, mandatory snapping, one viewport-width pane per page,
  `scroll-snap-stop: always`). Keep desktop resizers, edge-add affordances,
  and fractional widths only on wider layouts.
- Mirror the snapped panel index in an accessible dot or numeric indicator;
  reset to the first panel after switching workspaces.
- If swiping past the final panel creates a new one, require reaching almost
  the complete creation page, guard against duplicate pending creation, and
  scroll to the new panel only after state confirms it exists.
- Put workspace switching, creation, close, shortcuts, and Console settings
  in one phone menu rather than compressing the desktop tab strip.

### Prevent layout failures

- Put `min-width: 0` and `min-height: 0` on nested flex/grid children.
  Assign scrolling to the smallest region that needs it; never let the whole
  page scroll horizontally.
- Truncate ids, model names, paths, and tab titles deliberately; preserve
  the distinguishing tail of a filesystem path.
- Hide secondary metadata before squeezing the primary task: a details
  popover or sheet instead of a wrapping header; at narrow widths secondary
  header actions move into a `DropdownMenu`.
- Keep state mounted when hiding modes if cursor, draft, selection, or
  scroll continuity matters. Unmount only when changing domain identity must
  reset it.

## Visual language and delivery

Surfaces, type, selection, icons, elevation, motion, and every number:
`ade/design-system`. Slots, scoped CSS, the build, the lint, registration and
hot reload: `ade/injectable-ui`.

## Preserve state through async work and reload

- Hydrate once, subscribe to changes, and unsubscribe on cleanup
  (`useWorkerLive` from `@iii-dev/console-ui/hooks` does this for
  fetch + trigger bindings + visible-tab poll).
- Use request ids, abort controllers, or monotonic tokens so an old response
  cannot overwrite a newer selection or edited value. Invalidate
  connection-test results as soon as any tested field changes.
- Key persisted UI state on `paneId` (fall back to `tabId`); `usePaneState`
  mirrors it to browser storage best effort. Guard dirty drafts before
  navigation and report them through `setDirty`.
- Expect script hot reload to dispose and remount slot components. Persist
  only state that must survive.

## Configuration forms

Treat the provider settings flow as the baseline for every worker
configuration: focused, status-aware, schema-respecting, responsive, and
host-owned at the persistence boundary.

### Keep the ownership boundary strict

The form receives a complete JSON draft and proposes a complete next draft
(`ConfigFormProps` / `ProviderConfigFormProps` in `index.d.ts`; only the
former carries `focusField`). The Console owns loading, baseline, dirty
comparison, merged client and server validation, navigation guard,
Save/Reset, mutation status, and the sticky save bar. Never save from the
component and never keep a second persistent copy of the draft. Call
`onChange` with immutable updates, preserve unknown keys and siblings,
unknown enum/adapter payloads and templates, and delete an optional key
(`delete next[key]`) to restore its default; display defaults without
materializing them. An opaque root is preserved like an opaque nested block
and requires an explicit conversion; never coerce it to `{}` to enter the
typed form.

### Use the shared form grammar

Every `host.configForms` implementation uses host-owned primitives. Do not
paint native inputs, selects, switches, buttons, or a private collection deck
to resemble the Console, and never render a raw JSON textarea.

- Structure ordinary settings as `SettingsSection` → `SettingsList` →
  `SettingsField`/`SettingsRow`.
- `SettingsField` for editable values: pass every prop supplied by its
  `renderControl` callback into `Input`, `Select`, `Selector`, `Switch`, or
  a domain wrapper. It generates the clickable label, description/error
  ARIA, `data-field`, and standard control width. Use `controlSize="fit"`
  with `layout="inline"` for intrinsic controls such as `Switch`.
- `SettingsRow` for values or actions that are not a single labelled field.
- `RawValueInput` for `${ENV}` templates and unknown/future scalars. It may
  suggest a typed literal, but conversion happens only after the user
  invokes `onUseLiteral`. A non-string opaque value still belongs in a
  `SettingsField` with an explicit conversion button so errors stay
  associated with the control.
- `Select` for finite choices, `Selector` for searchable ones. Both support
  `id`, `name`, and `data-field`; a native `<select>` with worker CSS is
  never the fallback.
- Worker CSS may arrange controls, constrain width, or apply mono to a
  machine-readable value. It must not override shared control color,
  border, radius, height, chevron, focus, disabled, or type styles.
- Put `data-settings-narrow-action` on standalone empty-state actions so
  they use the shared narrow target rule.

For a collection whose item opens a meaningful sub-form, use `SettingsDeck`.
Its `overview` composes `Panel` + `List`/`ListItem`; its `detail` holds the
selected item's settings. `open` selects exactly one level at every width.
The deck focuses the pushed heading and restores the originating row on
Back. Keep selection by a stable domain key and set it to `null` when the
item is removed. For a host deep link, open the requested item first, focus
the exact `data-field`, and temporarily disable `autoFocusDetail`; encode the
host path as `focusField.map(String).join('.')`, use that dotted value in
`SettingsField.field`, and consume each request once after the deck content
mounts. Put `data-settings-deck-fallback` on the surviving overview action
that should receive focus when a removed item's row disappears.

```tsx
<SettingsField
  id="redis-url"
  field="adapter.config.redis_url"
  label="Redis URL"
  error={errors?.get('/adapter/config/redis_url')}
  renderControl={(controlProps) => (
    <Input {...controlProps} value={redisUrl} onChange={setRedisUrl} />
  )}
/>

<SettingsDeck
  open={activeId !== null}
  title={activeItem?.label ?? 'Connection'}
  backLabel="Connections"
  overview={<ConnectionList onOpen={setActiveId} />}
  detail={activeItem ? <ConnectionSettings item={activeItem} /> : null}
  onBack={() => setActiveId(null)}
/>
```

`database` is the canonical resource-deck example; `cron` is the canonical
small settings form.

### Use an anatomy that answers operator questions

1. Start with identity and live status only when it changes what the
   operator should do: connected/unconfigured, available/unloaded,
   model/resource count, active adapter, or restart required.
2. Put authentication or connectivity first. Explain where credentials live
   and provide a test/check action when the worker can verify them.
3. Group domain settings by mental model, not schema nesting: one section
   label and a quiet grouped surface for related rows.
4. Explain defaults and operational units beside the field; translate raw
   milliseconds, bytes, or token caps into human-readable echoes
   (`formatDuration`, `formatBytes` from `@iii-dev/console-ui/format`).
5. State when a setting hot-applies, applies on the next request, or requires
   a worker restart. Reveal advanced settings progressively.
6. End with inline root errors if no field can own them; keep field errors
   next to their controls.

There is no generic schema-form fallback: use the schema for draft
validation, never for UI generation. Even a simple configuration gets
purpose-written labels, grouping, defaults, and reload semantics.

### Handle secrets and authentication safely

- Never render a plaintext API-key field in provider configuration. If the
  provider declares a credential environment variable, show its exact name
  in a copyable mono token and explain that the key belongs in the runtime
  environment, outside stored configuration.
- If no credential variable exists, authentication is provider-owned: show
  its OAuth, device, CLI, local app, or companion-login instructions and
  expose a safe check/refresh action through the provider worker.
  Distinguish API-key providers from subscription/login providers explicitly.
- Do not treat `configured === false` as decisive for provider-owned auth; a
  discovered model catalog is authoritative evidence the provider works.
  Interpret `available === false` as worker unavailable, not merely missing
  credentials; keep the two messages distinct.
- After a successful host save, let the host refresh provider and model state.

### Make fields robust

- Derive visibility from the registered schema; do not expose fields the
  worker cannot accept. Render optional overrides with an explicit enable
  switch when property presence changes semantics; switching off deletes
  the key.
- Parse numbers without committing `NaN`; keep the empty state `undefined`
  when it means "use default"; apply schema min/max; use `inputMode` where
  appropriate.
- Give every control a stable label/id pair. Help text is for consequences,
  not to repeat the label.
- Map errors by JSON Pointer, surface them with `role="alert"`, clear stale
  server errors after edits, and keep Save disabled while client validation
  fails. Honor `focusField`: escape the selector segment, focus the matching
  element, and scroll it to the center.
- Guard renames or identity edits until blur/explicit commit so intermediate
  text cannot collide with sibling keys. For async tests, show checking,
  success with useful facts, and a concise error; ignore completion if the
  value changed or the component unmounted.

### Make configuration responsive

- Use the centered contained column for ordinary forms; request
  `{ layout: 'full' }` only for workbench-style configuration that owns its
  scrolling.
- Size controls from the pane, not the viewport: Back, section/row actions
  and field controls meet the touch target in a narrow split pane even on a
  wide desktop window (`ade/design-system` › Numbers). Phones get phone
  text size, stacked action buttons, and readable help text; desktop
  compacts controls without changing information architecture.
- Keep the host save bar sticky and always reachable; never cover it with
  internal scrolling.
- In a model-picker sheet or dropdown, keep provider configuration inside
  the current navigation surface and run the dirty guard before back or
  close.

## Validate the result

Static and delivery checks (build, lint, manifest, hot reload, trigger
renderer fallthrough): `ade/injectable-ui` › Testing.

### Interaction matrix

- A phone viewport, a narrow desktop split pane, and a wide pane (widths:
  `ade/design-system` › Numbers); left and right split positions; multiple
  horizontal phone panels; light and dark themes.
- Touch, pointer, keyboard-only, visible focus, reduced motion; neutral
  selected rows, cards, tabs, chips, and segments in both themes;
  responsive transitions and immediate high-frequency updates.
- Long names, paths, model ids, descriptions, and payloads; loading, empty,
  unavailable, unconfigured, success, error, reconnect, and hot-reload states.
- Sheet back/close, overlay dismissal, native browser Back where applicable,
  and dirty-draft confirmation; screen-reader names, roles, live status
  (`LiveRegion`), progress values, and selected state.
- No horizontal page overflow and no content hidden behind safe areas or the
  sticky save/composer regions.

### Definition of done

Finish only when the primary task is obvious at every width; every phone
action is reachable without hover; page and configuration state cannot be
lost silently; async work cannot overwrite newer intent; secrets never appear
in editable provider configuration; the host still owns validation and
persistence; targets, text sizes and icons match `ade/design-system` ›
Numbers; and `ade/injectable-ui` › Definition of done holds.
