---
name: design-console-ui
description: Design, implement, or review iii Console interfaces across mobile, narrow split panes, and desktop, including native Console surfaces, runtime-injected worker pages and renderers, worker configuration forms, and provider-owned configuration or authentication flows. Use for responsive Console UX, injectable React UI, configuration editors, model/provider setup, touch and keyboard accessibility, design-token styling, UI delivery, hot reload, or validation of a worker's Console experience.
---

# Design iii Console UI

Create operator interfaces that feel native to the Console, preserve host-owned state and safety boundaries, and remain fully usable on phones, narrow split
panes, and wide desktops. Treat visual quality, responsive behavior, accessibility, state integrity, delivery, and failure handling as one contract.

Keep this skill self-contained. Do not depend on another guide to supply a missing rule. Inspect only the target implementation and its current public API
when integrating a change; do not invent components, slots, or props.

## Work in this order

1. Inventory the behavior, data, states, actions, dirty drafts, async work, and existing constraints before changing markup.
2. Choose one information architecture for the primary task. Define the wide flow and the narrow flow separately.
3. Decide whether the surface belongs to the Console itself, a full injected page, a function renderer, a worker configuration form, a provider configuration form, or a compact chat slot.
4. Keep authoritative data, validation, persistence, and navigation guards in the host. Let an injected worker own presentation and worker-specific calls.
5. Build from shared primitives and design tokens. Add the least scoped CSS needed for the domain.
6. Exercise loading, empty, unavailable, unconfigured, dirty, saving, saved, error, reconnect, and stale-response states.
7. Verify phone, narrow-pane, and desktop behavior in both themes with touch, mouse, and keyboard before declaring the UI complete.

## Model the Console as responsive workspace software

Do not shrink desktop UI into a phone. Change the interaction model when the
available width changes.

### Use the correct width signal

- Use the Console chrome breakpoint at 640 px for viewport-wide native behaviors: phone UI below it and desktop UI at or above it.
- Use the component's own width for content hosted inside a workspace pane. A split desktop pane can be narrower than a phone viewport. Measure the container with `ResizeObserver`, or use CSS container queries for visual-only changes.
- Derive container thresholds from the minimum usable content width. Never copy a breakpoint without checking the target content.
- Ignore zero-width hidden panes, measure synchronously on ref attachment, and disconnect observers when refs change.

### Change structure on phones

- Replace side-by-side master/detail with a drill-in sequence: list → detail,
  scope → resource → value, or settings → category → choice.
- Render one primary page at a time and provide a visible, labelled back action.
- Replace competing dropdowns and dialogs with one bottom sheet and an in-place
  navigation stack. Never open a second portal on top of the sheet for a
  sub-selection.
- Keep selector logic presentation-independent so desktop dropdowns and mobile
  sheet pages use the same options, selected value, disabled rules, and change
  callbacks.
- Keep dangerous confirmation as another page in the same sheet. Do not bypass
  confirmation because the mobile presentation changed.
- Run the same unsaved-change guard for back, close, overlay dismissal, and
  sheet teardown.

### Size mobile interaction deliberately

- Make primary rows at least 56 px high and icon actions at least 48 × 48 px.
  A compact desktop control may shrink to 36–40 px.
- Keep form inputs, selects, textareas, and editable composer text at 16 px on
  phones to prevent browser zoom. Compact them to 13–14 px only on wider UI.
- Give the phone composer roughly 96 px of initial editor height; compact it to
  roughly 56 px on desktop.
- Never hide an essential action behind hover on coarse pointers. Use
  `pointer-fine` only for hover-only disclosure; keep the action visible on
  touch.
- When a small visual icon must remain compact, enlarge its invisible hit area
  to at least 48 px without changing layout.
- Keep focus rings visible, add an accessible name to every icon-only action,
  mark decorative icons `aria-hidden`, and expose selected state with
  `aria-pressed`, `aria-current`, radio semantics, or a checkmark—not color
  alone.

### Compose mobile sheets correctly

- Use a modal overlay, 12 px side/bottom inset, rounded raised panel, drag
  handle, explicit 48 px close target, and a maximum height based on
  `100dvh`.
- Add bottom padding using `env(safe-area-inset-bottom)`.
- Keep the page header fixed and give only the page content
  `overflow-y: auto; overscroll-behavior: contain`.
- Use a stack with `push`, `back`, and `reset`; avoid duplicate consecutive
  pages and reset after a successful close.
- Use grouped rows with a strong label, quiet current value, optional icon,
  and chevron. Use radio-style option rows for mutually exclusive choices.
- Avoid autofocus that opens the mobile keyboard as soon as a sheet appears;
  retain keyboard-first autofocus in desktop popovers where useful.

### Compose mobile workspaces correctly

- Turn multiple panes into full-width horizontal snap pages on phones. Keep
  desktop resizers, edge-add affordances, and fractional widths only on wider
  layouts.
- Use `snap-x`, mandatory snapping, one viewport-width pane per page, and
  `scroll-snap-stop: always` for deliberate swipes.
- Mirror the snapped panel index in an accessible dot or numeric indicator.
  Reset to the first panel after switching workspaces.
- If swiping past the final panel creates a new one, require reaching almost
  the complete creation page, guard against duplicate pending creation, and
  scroll to the new panel only after state confirms it exists.
- Preserve a defensive persistence ceiling, but do not impose the old
  three-panel desktop limit on horizontally scrollable mobile workspaces.
- Put workspace switching, creation, close, shortcuts, and Console settings in
  one phone menu rather than compressing the desktop tab strip.

### Prevent layout failures

- Put `min-width: 0` and `min-height: 0` on nested flex/grid children.
- Assign scrolling to the smallest region that needs it. Avoid whole-page
  horizontal scrolling.
- Truncate ids, model names, paths, and tab titles deliberately. For filesystem
  paths, preserve the distinguishing tail when appropriate.
- Hide secondary metadata before squeezing the primary task. Move it into a
  details popover or sheet instead of allowing the header to wrap.
- Keep state mounted when hiding modes if cursor, draft, selection, or scroll
  continuity matters. Unmount only when changing domain identity must reset it.

## Apply the Console visual language

### Build hierarchy with surfaces

- Use the surface ramp in this order: `--color-bg`, `--color-sidebar`,
  `--color-panel`, `--color-panel-raised`, `--color-surface`,
  `--color-surface-hover`, `--color-surface-selected`, and
  `--color-surface-active`.
- Use `--color-ink`, `--color-ink-faint`, and `--color-ink-ghost` for content
  hierarchy. Never use ghost ink for load-bearing text.
- Use `--color-alert`, `--color-warn`, and `--color-ok`, plus their muted
  variants, only for meaningful status. Reserve `--color-accent` for primary
  actions, form focus, live activity, or semantic domain data.
- Use `--color-edge` for structural boundaries and `--color-rule-focus` for
  focus. Ordinary rule tokens may be transparent; do not rely on divider soup
  to create hierarchy.
- Use `--shadow-raised` and `--shadow-floating` only when a surface actually
  leaves its layer.
- Keep the system radius near 6 px. Reserve full rounding for status dots,
  pills, and circular primary actions.
- Render selection with `--color-surface-selected`, stronger
  `--color-ink`, and an optional neutral `--color-edge`. Never change a
  selected row, card, tab, chip, or segment to accent text or an accent edge.

### Use type by meaning

- Use `--font-sans` for titles, labels, prose, menus, settings, status, and
  explanations.
- Use `--font-mono` only for machine-produced ids, paths, schemas, counts,
  data values, and payloads. Never make a whole panel or its controls mono.
- Use `--font-code` for source, structured payloads, terminal output, and
  editors.
- Prefer sentence/title case and direct product language. Do not apply CSS
  `lowercase` or `uppercase` transforms to tabs, buttons, menus, or forms.
- Keep phone body and control copy near 16 px. Use 17–18 px semibold titles,
  13–14 px desktop body copy, and 10–12 px metadata where contrast remains AA.
- Keep explanatory prose within roughly 60–72 characters per line.

### Avoid generic generated UI

- Choose one dominant archetype: catalog/detail, data workbench, document
  editor, hierarchy explorer, or settings flow.
- Avoid card soup, ornamental gradients, glows, giant centered headings,
  excessive badges, random accent colors, repeated descriptions, and controls
  floating in empty space.
- Use cards for cohesive entities or auth/status summaries, not for every
  field. Use quiet grouped rows and section spacing for ordinary settings.
- Put page actions in the page header, resource actions beside resource
  identity, and work actions in the nearest toolbar.
- Keep one clear primary action at the point of work. Move rare actions into a
  menu.
- Render loading, empty, error, and success in the space the content will
  occupy so the silhouette remains stable.

### Reuse the public UI contracts

- Compose page chrome and the shared `List`/`ListItem`, `Card`, `Panel`,
  `Chip`, `IconButton`, semantic `Table` parts, line `Tabs`, and
  `SegmentedControl` primitives before adding local structure. Use stable
  `uiClasses` recipes when a worker needs semantic markup without another
  React wrapper.
- Build simple tables as `TableViewport` → `TableFrame` → `Table`, with the
  shared semantic sections, rows, headings, and cells. Use natural-case sans
  headers, horizontal dividers, comfortable page density or compact chat
  density, and mono only for technical cell values. Only interactive rows get
  hover treatment; selected rows remain neutral.
- Use `TabsList variant="line"`/`TabsTrigger` or `SegmentedControl
  variant="tabs"` for peer content views. Shared tabs use a bottom rule,
  neutral active underline, 600 weight, natural case, and a semantic 16 px
  icon by default. Reserve `variant="radio"` and its surface track for a
  persisted mutually exclusive choice; do not carry private boxed-tab CSS.
- Use application icons at the shared 16 px baseline. Never author an icon
  usage, component default, or root SVG below 16 px. Use `IconButton` for icon-only actions so the glyph
  keeps an accessible name and shared tooltip.
- Use `Selector` for searchable single choice, including grouped or disabled
  options, caller-owned async search, loading/empty/error/validation states,
  and explicitly enabled free-form creation. Use `Select` for a small finite
  non-searchable list.
- Use shared `Tooltip` parts or `IconButton`; do not create local hover timers,
  collision logic, or tooltip portals.
- Keep a local selector only when its interaction is materially different,
  such as hierarchical drill-in, multi-select, or a persistent command
  palette. Document the exception in the Console UI conformance inventory.
- Use `--motion-duration-*` and `--motion-ease-*`, or the shared motion recipe
  classes, for control, panel, and overlay transitions. Streaming text,
  rapidly updating meters, and pointer-following geometry update immediately.

## Build runtime-injected worker UI

Use injected UI when a worker must ship its own page, function result,
configuration body, provider authentication, or compact chat status without
rebuilding the Console. Injected scripts run in the Console's React tree and
origin; the wrapper scopes styling but is not a security sandbox.

### Understand the delivery contract

- Register one content function that receives `{ path }` and returns
  `{ content, content_type? }`.
- Register one Message-path trigger per asset: `console:script` for ESM and
  `console:style` for CSS. Never register the tab-only `console:assets` type.
- Use lowercase path segments containing only letters, digits, `.`, `_`, and
  `-`; disallow a leading slash and `.` or `..` segments; keep paths at most
  512 characters; match `.js`/`.css` to the trigger kind.
- Make the first path segment the worker name. It becomes the
  `data-iii-ui` scope and human attribution.
- Keep every asset below 8 MiB; normal slot bundles should be tens of KiB.
- Treat identical content hashes as no-ops. Re-registering changed bytes at
  the same path is deployment and hot reload.
- Use Message-path registration so disconnect garbage-collects the UI and SDK
  reconnect replays it. Do not use durable engine registration.

### Build without a second runtime

Create a private workspace UI package with React/TypeScript, esbuild, and `@iii-dev/console-ui` as a `workspace:*` dependency. Make its build run TypeScript without emitting, then run esbuild. Bundle as ESM and mark exactly these specifiers external:

```js
external: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@iii-dev/console-ui']
```

Forgetting React creates an invalid second hook dispatcher. Leaving the shared
package bundled breaks the runtime contract. Do not import unsupported bare
React subpaths through dependencies.

Never bundle Monaco, CodeMirror, a diff renderer, or an ANSI parser. Use the
shared `CodeEditor`, `FileDiff`, `AnsiText`, `TerminalStream`, and
`TerminalCommandLine` surfaces.

For a Rust worker, gate the UI dependency and registration behind a `console-ui` feature when UI-free builds matter. Build generated assets only when the feature is enabled and sources are newer or output is missing. Allow `SKIP_UI_BUILD` only when embedded assets already exist. Embed and register both assets through one builder:

```rust
ConsoleUi::new("mywork")
    .script("mywork/page.js", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js")))
    .style("mywork/styles.css", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css")))
    .register(&iii);
```

Test that the script contains the expected registration and the stylesheet contains the worker scope. For live development, rebuild output on save and let the worker watch it through `III_<WORKER>_UI_WATCH`.

### Register through `setup(host)`

Default-export one setup function and make every contribution through its
host so disposal on reload is automatic:

```tsx
import {
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
} from '@iii-dev/console-ui'

function WorkerPage({
  host,
  panelSide,
  onRequestClose,
}: PageRenderProps & { host: Host }) {
  return (
    <PageShell className="mywork-ui">
      <PageHeader
        title="Mywork"
        description="Operator workspace"
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageMain>{/* primary task */}</PageMain>
      </PageBody>
    </PageShell>
  )
}

export default function setup(host: Host) {
  host.pages.register({
    id: 'mywork-manager',
    title: 'Mywork',
    render: (props) => <WorkerPage host={host} {...props} />,
  })
  host.functionTriggers.register(createMyworkRenderer(host))
  host.configForms.register('mywork', MyworkConfigurationForm)
  host.providerConfigForms?.register('my-provider', MyProviderForm)
  host.chat?.registerSessionChip({ id: 'mywork-status', render: StatusChip })
  host.chat?.registerTurnSummary?.({ id: 'mywork-summary', render: TurnSummary })
}
```

Register only implemented surfaces. Feature-detect optional namespaces for
older Console versions.

### Use each slot for its job

- `host.pages.register`: create a whole workspace page. Always keep
  `PageShell` and `PageHeader`; wire `onRequestClose`; use `PageBody`,
  `PageSidebar`, and `PageMain` when their structure fits.
- Page render props: use `panelSide` only to place wide navigation on the
  outside edge; key per-tab state by `tabId`; react to live `workingDir`; use
  `conversationId` for exact session subscriptions; treat `panelContext` as
  ephemeral JSON navigation context.
- `host.panels?.open`: place or reuse a registered contextual page beside
  chat. Pass opaque ids for large or sensitive content and fetch details from
  the worker.
- `host.functionTriggers.register`: match only the worker's function ids and
  return `null` for unsupported or error shapes so default renderers can run.
  Mark `metadata.display` only for successful rich artifacts worth keeping
  expanded.
- Implement `redactRaw` whenever input/output can contain secrets. Make it a
  pure, non-mutating, total, cycle-safe deep walk over values and keys. A
  render card that hides a secret has not protected the raw JSON tab or copy
  action.
- `host.configForms.register`: replace only one worker's form body. Use
  contained layout by default; request full layout only for a workbench that
  owns all internal scrolling.
- `host.providerConfigForms?.register`: replace exactly one provider's editor
  in the model picker. Use it for OAuth, device flow, local/companion login,
  or genuinely provider-specific settings.
- `host.chat?`: render compact per-session status only. Fetch worker-owned data
  through `host.iii`; do not turn the header into a dashboard.
- `host.iii`: call worker functions, hydrate state, subscribe to events, and
  unregister handlers on teardown. Prefix per-tab event handler ids with
  `iii::` when they should stay out of the trace feed.

### Scope every injected style

Prefix every selector with the worker wrapper:

```css
[data-iii-ui="mywork"] .mywork-ui-main {
  min-width: 0;
  min-height: 0;
  overflow: auto;
  color: var(--color-ink);
  background: var(--color-panel);
}

@media (prefers-reduced-motion: reduce) {
  [data-iii-ui="mywork"] .mywork-ui-animated { transition: none; }
}

@keyframes mywork-ui-flash { /* prefix global keyframe names */ }
```

Never use unscoped `:root`, `html`, `body`, `*`, bare element selectors, or
`@font-face`. Never hardcode theme colors. Do not use Tailwind utilities in
injected markup because worker classes are absent from the Console's compiled
Tailwind output. Use shared components and `uiClasses` recipes plus scoped CSS
for domain-specific layout. Shared motion recipes honor the Console's global
reduced-motion contract; custom animation still needs a scoped override.

Shared `Dialog`, `DropdownMenu`, `Select`, `Selector`, `Tooltip`, and
`BottomSheet` portals carry the current worker scope automatically. If custom
domain UI portals directly into `document.body`, stamp
`data-iii-ui="mywork"` on that portal root.

### Preserve state through async work and reload

- Hydrate once, subscribe to changes, and unsubscribe on cleanup.
- Use request ids, abort controllers, or monotonic tokens so an old response
  cannot overwrite a newer selection or edited value.
- Invalidate connection-test results as soon as any tested field changes.
- Expect script hot reload to dispose and remount slot components. Persist only
  state that must survive, key it by stable tab/session identity, and treat
  browser storage as best-effort.
- Register the new trigger before unregistering its old handle to avoid a
  zero-trigger flash and replay-map growth.

## Design ideal worker configuration

Treat the recent provider settings flow as the baseline for every custom
worker configuration: focused, status-aware, schema-respecting, responsive,
and host-owned at the persistence boundary.

### Keep the ownership boundary strict

The worker form receives a complete JSON draft and proposes a complete next
draft. The Console owns loading, baseline, dirty comparison, merged client and
server validation, navigation guard, Save/Reset, mutation status, and sticky
save bar.

```ts
interface ConfigFormProps {
  id: string
  schema: Record<string, unknown> | null
  value: JsonValue
  onChange(next: JsonValue): void
  errors?: ReadonlyMap<string, string>
  focusField?: readonly string[]
}

interface ProviderConfigFormProps {
  providerId: string
  schema: Record<string, unknown> | null
  value: JsonValue
  onChange(next: JsonValue): void
  errors?: ReadonlyMap<string, string>
  configured?: boolean
  available?: boolean
  modelCount: number
}
```

Only `ConfigFormProps` carries `focusField`; provider forms do not receive or implement deep-link focus.

Never save from the injected component and never maintain a second persistent
copy of the draft. Call `onChange` with immutable updates, preserve unknown
keys, and delete optional keys to restore provider/worker defaults:

```ts
function patch(current: Record<string, JsonValue>, key: string, next?: JsonValue) {
  const updated = { ...current }
  if (next === undefined) delete updated[key]
  else updated[key] = next
  return updated
}
```

### Use a configuration anatomy that answers operator questions

1. Start with identity and live status only when it changes what the operator
   should do: connected/unconfigured, available/unloaded, model/resource count,
   active adapter, or restart required.
2. Put authentication or connectivity first. Explain where credentials live
   and provide a test/check action when the worker can verify them.
3. Group domain settings by mental model, not schema nesting. Use one section
   label and a quiet grouped surface for related rows.
4. Explain defaults and operational units beside the field. Translate raw
   milliseconds, bytes, or token caps into human-readable echoes.
5. State when a setting hot-applies, applies on the next request, or requires a
   worker restart. Do not make operators infer reload semantics.
6. Reveal advanced settings progressively. Keep the common path short.
7. End with inline root errors if no field can own them; keep field errors next
   to their controls.

Use the generic schema form when labels, descriptions, types, and validation
already produce a clear UI. Add a custom worker form when configuration needs
conditional fields, domain grouping, connection tests, humanized units,
restart semantics, complex dictionaries/arrays, or worker-specific actions.

### Handle secrets and authentication safely

- Never render a plaintext API-key field in provider configuration.
- If the provider declares a credential environment variable, show its exact
  name in a copyable mono token and explain that the key belongs in the runtime
  environment, outside stored configuration.
- If no credential variable exists, treat authentication as provider-owned.
  Show its OAuth, device, CLI, local app, or companion-login instructions and
  expose a safe check/refresh action through the provider worker.
- Distinguish API-key providers from subscription/login providers explicitly so
  an operator never pastes the wrong credential into the wrong surface.
- Do not treat `configured === false` as decisive for provider-owned auth. A
  successfully discovered model catalog is authoritative evidence that the
  provider is usable.
- Interpret `available === false` as worker unavailable, not merely missing
  credentials. Keep unavailable and unconfigured messages distinct.
- After a successful host save, let the host refresh provider and model state.

### Make fields robust

- Derive visibility from the registered schema; do not expose fields the
  worker cannot accept.
- Render optional overrides with an explicit enable switch when property
  presence changes semantics. Turning the switch off must delete the key.
- Parse numbers without committing `NaN`. Preserve the empty state as
  `undefined` when it means “use default.” Apply schema min/max constraints.
- Use `inputMode="numeric"` or `inputMode="url"` where appropriate.
- Give every control a stable label/id pair. Use help text for consequences,
  not to repeat the label.
- Map errors by JSON Pointer, surface them with `role="alert"`, clear stale
  server errors after edits, and keep Save disabled while current client
  validation fails.
- Honor `focusField`: escape the selector segment, focus the matching element,
  and scroll it to the center.
- Guard renames or identity edits until blur/explicit commit so intermediate
  text cannot collide with sibling keys.
- For async tests, show checking, success with useful facts, and concise error;
  ignore completion if the underlying value changed or the component unmounted.

### Make configuration responsive

- Use a centered contained column for ordinary forms and a full-height layout
  only for workbench-style configuration.
- On phones, use 16 px text, 48 px controls, 56 px option rows, 16 px page
  padding, stacked action buttons, and readable help text.
- On desktop, compact controls without changing information architecture.
- Keep the host save bar sticky and always reachable. Never cover it with
  internal scrolling.
- In a model-picker sheet or dropdown, keep provider configuration inside the
  current navigation surface and run the dirty guard before back or close.

## Validate the result

### Static and delivery checks

- Type-check and build the UI package.
- Confirm generated assets are non-empty ESM/CSS, React remains external, and
  no editor/diff/ANSI runtime was bundled.
- Confirm embedded assets register successfully and scoped CSS contains the
  worker wrapper.
- Inspect the UI manifest: require expected paths, non-empty hashes, enabled
  worker state, and an empty warnings list.
- Fetch each asset, verify a changed build produces a changed hash, and verify
  disconnect/reconnect does not create duplicate registrations.

### Interaction matrix

Exercise all of the following:

- 320–430 px phone viewport, narrow desktop split pane, and wide pane;
- left and right split positions and multiple horizontal phone panels;
- light and dark themes;
- touch, pointer, keyboard-only, visible focus, and reduced motion;
- neutral selected rows, cards, tabs, chips, and segments in both themes;
- responsive control/panel/overlay transitions and immediate high-frequency
  updates;
- long names, paths, model ids, descriptions, and payloads;
- loading, empty, unavailable, unconfigured, success, error, reconnect, and
  hot-reload states;
- sheet back/close, overlay dismissal, native browser Back where applicable,
  and dirty-draft confirmation;
- screen-reader names, roles, live status, progress values, and selected state;
- no horizontal page overflow and no content hidden behind safe areas or the
  sticky save/composer regions.

### Definition of done

Finish only when the primary task is obvious at every width; every mobile
action remains reachable without hover; page and configuration state cannot be
lost silently; async work cannot overwrite newer intent; styles remain scoped
and token-based; secrets never appear in editable provider configuration; the
host still owns validation and persistence; the manifest is warning-free; and
the browser reports no injected-UI errors. Content tabs use the shared line
recipe with natural casing and default 16 px icons; UI chrome is sans; mono is
limited to machine-readable content; and no application icon is authored below
16 px.
