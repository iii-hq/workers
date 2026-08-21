# Custom trigger activity components

How a worker supplies source-specific UI for trigger registration, firing,
and retirement without moving the activity lifecycle into worker code.

Use this slot when generic JSON does not explain a trigger source well. A
cron worker can show a readable schedule, for example, while the Console
continues to own the surrounding **Trigger fired** activity, destination,
delivery result, lifecycle state, controls, and raw details.

The public contract is declared in
[`packages/console-ui/index.d.ts`](../../../packages/console-ui/index.d.ts).
The cron worker is the reference implementation:
[`cron/ui/src/trigger-activity/`](../../../cron/ui/src/trigger-activity/).

## Ownership boundary

The renderer owns only the source-specific section:

- interpret `triggerType` and `config`;
- explain worker-specific fields and, when useful, source payload details;
- return `null` for another trigger type or a shape it cannot safely render.

The host always owns:

- message chrome and the **Trigger fired** label;
- notify/call delivery and the target function;
- active, consumed, unbound, expired, invalidated, and failed states;
- once/max-fire counters and lifecycle controls;
- raw JSON and the generic source fallback.

Do not reproduce host-owned information in the injected component. In
particular, a once trigger that fires and retires remains one activity: the
host renders its consumed state rather than asking the worker to emit a
second unbind card.

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
`lifecycle` are context for source rendering, not permission to replace the
host's status UI.

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
  }
}

export default function setup(host: Host) {
  host.triggerRenderers?.register(createRenderer())
}
```

Registrations are tried in registration order. The first non-null render
wins; if all renderers return `null`, the host shows its generic source
section. A throwing `isMatch` is treated as no match, and rendering failures
are fenced so one worker cannot break the activity feed.

The loader disposes registrations when the script reloads or the worker
disconnects. Always register through the supplied `host`; do not keep a
parallel global registry.

## Design the source section

Keep the component compact enough for chat and trace surfaces:

- lead with the human interpretation, then preserve the exact machine value;
- use sans-serif text for labels and prose, mono only for expressions, ids,
  paths, or payload values;
- state units and timezones explicitly;
- use Console tokens and scope every selector under
  `[data-iii-ui='<worker>']`;
- avoid buttons unless the action is truly source-owned;
- prefer an honest raw value over a translation that hides important
  semantics.

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
- the same source section across registration, fired, and retirement kinds;
- persistent and once lifecycle records without duplicating host status;
- generic fallback when the worker UI is disabled or disconnected;
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
