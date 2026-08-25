# Custom trigger activity components

How a worker customizes trigger registration, firing, and retirement from a
small source interpretation through a complete timeline/detail presentation.

Use this slot when generic JSON does not explain a trigger source well. A cron
worker can show a readable schedule, replace the complete expanded Terminal
tab, or supply the compact event row shown in chat. The Console continues to
own the clickable disclosure and a raw-data fallback.

The public contract is declared in
[`packages/console-ui/index.d.ts`](../../../packages/console-ui/index.d.ts).
The cron worker is the reference implementation:
[`cron/ui/src/trigger-activity/`](../../../cron/ui/src/trigger-activity/).

## Layered ownership

Choose the smallest hook that expresses the worker's meaning:

- `tryRender` interprets the source section inside the generic detail view;
- `tryRenderDetails` replaces the complete expanded Terminal tab;
- `tryRenderDisplay` replaces the compact clickable timeline content;
- `redactRaw` filters registration and fire values before raw display/copy.

Every hook is registration-ordered and may return `null` to fall through. A
worker that only needs a schedule should keep using `tryRender`; a worker with
a domain-specific lifecycle or event artifact can own the detail/display
slots. Do not add interactive children to `tryRenderDisplay`, because the host
wraps it in the disclosure button.

The host always owns:

- the click target, expanded/collapsed state, focus behavior, and animation;
- the compact fallback (status icon plus event text);
- the generic detail view when no complete override wins;
- the Raw JSON tab after worker-declared redaction;
- renderer isolation, ordering, hot-reload disposal, and error boundaries.

When overriding the complete detail, the worker becomes responsible for
showing every lifecycle/delivery fact its operator needs. A once trigger that
fires and retires remains one activity; never emit a second unbind card.

## Event copy: label versus action

Harness registrations accept both concepts:

```json
{
  "trigger_type": "on-message",
  "config": { "scope": "explorer" },
  "label": "explorer-messages",
  "metadata": { "action": "new Explorer message received" }
}
```

`label` is the stable identity of the binding. `metadata.action` is the short
user-facing description of what a future fire means. The harness exposes the
action as data before anything happens and persists it on the fired record,
but the default registration and active-binding UI continues to show `label`.
`action` becomes visible only when the trigger fires: the compact event row
shows a status mark and `action`, then falls back to `label`, a state scope/key,
or `triggerType`. Clicking it opens the full card already expanded. Keep action
concise, standalone, and in the same language as the surrounding product UI.
Raw JSON still retains the declared metadata for inspection; it is not part of
the user-facing event copy.

## Message contract

The host normalizes registration, fired, and retirement records before
dispatching them:

```ts
interface TriggerActivityMessage {
  id: string
  kind: 'registration' | 'fired' | 'retirement'
  triggerType: string
  config?: unknown
  label?: string
  action?: string
  conditions?: readonly unknown[]
  delivery:
    | { kind: 'notify' }
    | { kind: 'call'; functionId: string }
  lifecycle: {
    state: 'active' | 'retired'
    once: boolean
    maxFires?: number
    expiresAt?: number
    fires: number
  }
  subscriptionId?: string
  triggerId?: string
  payload?: unknown
  firedAt?: number
  note?: string
  outcome?:
    | 'delivered'
    | 'delivery_failed'
    | 'skipped'
    | 'expired'
    | 'unregistered'
    | 'invalidated'
  retirementReason?:
    | 'once_consumed'
    | 'max_fires'
    | 'expired'
    | 'unregistered'
    | 'invalidated'
    | 'exhausted'
}
```

`config` and `payload` remain `unknown` because their schemas belong to the
trigger worker. Parse them defensively. Fields under `delivery` and
`lifecycle` are available to both source and complete-detail renderers.

Match `triggerType`, not the function used to register the trigger. Several
sources travel through the same `engine::register_trigger` function; the
inner type (`cron`, `state`, or a worker-defined value) is the source
identity.

## Register a renderer

Default-export `setup(host)` from the worker's injected script and
feature-detect the slot for consoles released before trigger activity
renderers:

```tsx
import type {
  Host,
  TriggerActivityMessage,
  TriggerActivityRenderer,
} from '@iii-dev/console-ui'

function readConfig(value: unknown): { expression: string } | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const expression = (value as Record<string, unknown>).expression
  return typeof expression === 'string' && expression.trim()
    ? { expression: expression.trim() }
    : null
}

function createRenderer(): TriggerActivityRenderer {
  return {
    id: 'my-worker/page.js#trigger-activity',
    isMatch: (triggerType) => triggerType === 'my-trigger',
    tryRender: (activity: TriggerActivityMessage) => {
      if (activity.triggerType !== 'my-trigger') return null
      const config = readConfig(activity.config)
      if (!config) return null
      return (
        <section aria-label="My trigger source">
          <span>Schedule</span>
          <code>{config.expression}</code>
        </section>
      )
    },
    tryRenderDisplay: (activity) => {
      const config = readConfig(activity.config)
      return config && activity.kind === 'fired'
        ? <span>{activity.action ?? 'schedule became due'}</span>
        : null
    },
    tryRenderDetails: (activity) => {
      const config = readConfig(activity.config)
      return config ? <MyCompleteTriggerDetails activity={activity} config={config} /> : null
    },
    redactRaw: redactMyTriggerSecrets,
  }
}

export default function setup(host: Host) {
  host.triggerRenderers?.register(createRenderer())
}
```

Each slot resolves independently in registration order. If every source hook
returns `null`, the host shows the generic source section; the same fallback
rule applies to display and details. A throwing `isMatch` is treated as no
match, and rendering failures are fenced so one worker cannot break the feed.

The loader disposes registrations when the script reloads or the worker
disconnects. Always register through the supplied `host`; do not keep a
parallel global registry.

## Design source, display, and details

Keep each surface appropriate to where it appears:

- keep `tryRenderDisplay` to one non-interactive, truncation-safe line;
- show `activity.action` only for `activity.kind === 'fired'`; registration
  and active-binding surfaces identify the binding by `label`;
- in source/details, lead with human interpretation, then preserve exact values;
- use sans-serif text for labels and prose, mono only for expressions, ids,
  paths, or payload values;
- state units and timezones explicitly;
- use Console tokens and scope every selector under
  `[data-iii-ui='<worker>']`;
- avoid buttons unless the action is truly source-owned;
- prefer an honest raw value over a translation that hides important
  semantics.

If raw data contains a secret, implement `redactRaw` as a pure,
non-mutating, total, cycle-safe transform. It applies to registration,
notification, and fire panes as well as their copy actions. A throw fails
closed to a withheld-value placeholder; it never restores the secret.

For cron, expressions with ranges, extensions, a pinned year, or combined
day-of-month/day-of-week rules should fall back to a “custom schedule” label
plus the exact expression unless the worker can describe them precisely.
Treat a wildcard step as “every N” only when it divides the field's full
period exactly; `*/7` minutes resets at the hour and is not a uniform
seven-minute interval.

## Suggested worker layout

```text
my-worker/
  build.rs
  src/ui.rs
  ui/
    page.tsx
    styles.css
    build.mjs
    package.json
    tsconfig.json
    src/trigger-activity/
      index.tsx
      parser.ts
      parser.test.ts
      renderer.test.tsx
```

Keep React, `react-dom`, `react-dom/client`, `react/jsx-runtime`, and
`@iii-dev/console-ui` external in the ESM build. Rust workers can embed and
register the script and stylesheet with the shared `iii-console-ui` crate.
The full delivery workflow lives in
[`docs/sops/injectable-console-ui.md`](../../../docs/sops/injectable-console-ui.md).

## Test matrix

At minimum, cover:

- exact trigger-type matching and fallthrough for another type;
- malformed/missing config returning `null`;
- common and complex source configurations;
- source, compact display, and complete details across the kinds each hook supports;
- persistent and once lifecycle records without duplicating host status;
- per-slot fallthrough and generic fallback when worker UI is disabled;
- `redactRaw` across nested values, copy paths, cycles, and thrown errors;
- light/dark themes, narrow chat panes, long ids, keyboard navigation, and
  worker reconnect/hot reload;
- a non-empty ESM bundle, external React, scoped CSS, and clean UI manifest.

For cron, run both layers:

```bash
pnpm --dir cron/ui test
pnpm --dir cron/ui build
cargo test --manifest-path cron/Cargo.toml ui
```

Then register a persistent cron trigger and a once cron trigger in the real
Console. Confirm that registration, firing, and retirement retain the custom
schedule section while the host alone renders delivery and lifecycle.
