# @iii-dev/native-ui

The SDK for a worker's **native app** in the iii app. A worker ships a small
TSX bundle; the iii app runs it in a QuickJS VM on the device (one VM per
worker, its own thread, a heap cap and a CPU deadline, no `eval`, no
network) and draws what it describes with its own Jetpack Compose (Android)
and SwiftUI (Apple) components. The script decides **what** appears; the app
decides **how and when** (layout, virtualized lists, gestures, text editing,
phone/tablet/Mac arrangement).

Design and wire contract: `ide/projects/worker-native-ui/design.md` (§4 the
capability object, §5 the protocol, §6 the catalogue, §10 this package).
The machine schema is [`catalog.json`](catalog.json); the ide repo keeps a
byte-identical copy. How a worker ships an app end to end:
[`docs/sops/native-ui.md`](../../docs/sops/native-ui.md).

**Linked by `workspace:*`, never published.**

## What is in the package

| Entry | What |
|---|---|
| `@iii-dev/native-ui` | `defineApp`, the components (`Column` … `RemoteScreen`), `createList` and `List`, the `iii` capability object, the hooks, `parseUrl`, `IiiError`, and the Solid primitives an app needs |
| `@iii-dev/native-ui/renderer` | The `solid-js/universal` renderer the JSX compiles against (the build wires it; apps never import it) |
| `@iii-dev/native-ui/build` | `buildNativeUi`: TSX → one IIFE bundle, with size limits, a smoke run and `--watch` |
| `@iii-dev/native-ui/testing` | `createTestHost`: the bundle in `quickjs-emscripten` with the device VM's globals, driven like the app drives it |
| `@iii-dev/native-ui/catalog.json` | The canonical schema |
| `@iii-dev/native-ui/tsconfig.app.json` | The compiler options an app extends (no DOM, JSX for Solid) |

## An app

```
<worker>/native-ui/
  package.json     "@iii-dev/native-ui": "workspace:*", "build": "node build.mjs", "test": "vitest run"
  tsconfig.json    { "extends": "@iii-dev/native-ui/tsconfig.app.json", "include": ["src"] }
  build.mjs        buildNativeUi({ entry: 'src/app.tsx', outfile: 'dist/native.js', ns: '<ns>' })
  src/app.tsx      defineApp({ pages: { root: … } })
```

```tsx
import { Button, Column, createSignal, defineApp, iii, Page, Text } from '@iii-dev/native-ui'

function Counter() {
  const [count, setCount] = createSignal(0)
  return (
    <Page title="Demo">
      <Column gap={12} padding={16}>
        <Text role="title">Count {count()}</Text>
        <Button kind="primary" onPress={() => setCount(count() + 1)}>
          Add
        </Button>
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Counter } })
```

That bundle mounts as the §5.7 counter of the design, byte for byte
(`conformance/counter.json`). [`examples/hello`](examples/hello) is a
slightly larger app (a list, a text field, buttons, a menu, storage) and the
known-good bundle to load in a host: `pnpm -F @iii-dev/native-ui build:example`
writes `examples/hello/dist/native.js`.

### Pages

`defineApp({ pages })` declares every page once; `root` is required. A page
is a component `(props: { page, params }) => JSX` or `{ component, modal?,
fullScreen? }`. The host mounts and unmounts page instances (one per card
of the app's deck); `iii.nav.push(name, params)` stacks a page and resolves
with the result its card leaves with (`usePage().pop(result)`), or `null`
after Back. `usePage()` gives the instance (`inst`, `name`, `params`) with
`pop` and `replace` bound to it.

### Components

One component per catalogue type, with the catalogue's names: `Column`,
`Row`, `Box`, `Scroll`, `Spacer`, `Divider`, `Text`, `Icon`, `Badge`,
`StatusDot`, `Spinner`, `Progress`, `EmptyState`, `Image`, `StatusPanel`,
`Button`, `IconButton`, `TextField`, `Switch`, `Segmented`, `Chip`, `List`,
`ListRow`, `Card`, `Page`, `ListDetail`, `Sheet`, `RemoteScreen`.

- **Props** are typed from the catalogue (`src/generated/props.ts`). Enums
  take their names (`role="title"`, `kind="primary"`); `null`/`undefined`
  means the default. Every component takes the common props `visible`,
  `testTag`, `a11yLabel`, `weight`, `width`, `height` (-1 = fill),
  `padding` (a number, `[v, h]` or `[top, end, bottom, start]`),
  `alignSelf`, and `ref` (a `NodeRef`, the anchor of `iii.ui.actionMenu`
  and the target of `iii.ui.focus`).
- **Events** are `on<Event>` props (`onPress`, `onLongPress`, `onChange`,
  `onSubmit`, `onSelect`, `onFocus`, `onDismiss`, `onRefresh`,
  `onEndReached`, `onFrame`, `onError`, `onKeyboard`, `onReady`). A handler
  subscribes the node; the host only sends subscribed events and only arms
  the gestures behind them (no long-press haptic without `onLongPress`).
- **Slots**: `children` is slot 0; the other slots are props named after
  them (`actions`, `top`, `bottom`, `leading`, `trailing`, `list`, `detail`,
  `emptyDetail`, `header`, `empty`, `footer`). Where slot 0 has another name
  (`EmptyState.actions`, `Page.body`, `Sheet.content`), that prop works too.
- **Text**: the JSX children of `Text`, `Button`, `Badge` and `Chip` become
  their `text` (`<Text>Count {n()}</Text>`). Text anywhere else is not shown
  (a warning in the log).
- `Switch` and `Segmented` show the person's change at once; if the app
  does not take it (the `checked`/`selected` prop stays), the SDK sends the
  app's value back.

### TextField: the text belongs to the host

`TextField` is uncontrolled (design §5.6): each edit of the person reaches
`onChange(text)`; `value` is only a write, sent with the count of edits the
field has seen, and the host drops it when the person typed again
meanwhile (and never writes during IME composition). Keep `value` in a
signal that `onChange` updates; writing back the text the host already has
sends nothing (it would move the cursor).

```tsx
const [url, setUrl] = createSignal('')
<TextField value={url()} keyboard="url" imeAction="go" onChange={setUrl} onSubmit={(text) => go(text)} />
```

### Lists: templates and data on the host

A `List` has no node per item. Its items live on the host
(`listSplice`), and up to four templates describe a row, with props bound
to item fields:

```tsx
interface Tab { key: string; kind: 0 | 1; title: string; url?: string }
const tabs = createList<Tab>({ key: 'key', kind: 'kind' })

<List
  source={tabs}
  templates={[
    (row) => (
      <ListRow title={row.title} subtitle={row.url} subtitleMono
        onPress={(item: Tab) => select(item.key)}
        trailing={<IconButton icon="x" label="Close tab" onPress={(item: Tab) => close(item.key)} />} />
    ),
    (row) => <Text role="micro" text={row.title} />,
  ]}
  empty={<EmptyState title="No tabs" />}
/>
```

- A template runs **once**; `row.title` is `{"$item": "title"}` on the wire
  (nested paths work: `row.meta.host`). The host does not format anything:
  compute the strings a row shows into the item.
- Handlers inside a template get the item first (found by the key of the
  host's `itemEvent`). Argument-less handlers are typed `(item?: any)`;
  inside a template, annotate the item (`(item: Tab) => …`).
- `createList({ key, kind?, items? })`: `items()` (reactive), `set(all)`
  (only what changed travels: the changed positions when keys keep their
  order, else one splice between the common head and tail), `splice`,
  `update(key, patch)`, `remove(key)`, `get(key)`. Treat items as immutable
  JSON; change them through the source.
- `TextField`, `List`, `RemoteScreen` and `Sheet` cannot sit in a template.

### The capability object

Everything outside the tree goes through `iii` (design §4.2), as a request
the host checks:

| | |
|---|---|
| `iii.call(fn, payload?, { timeoutMs? })` | A function of the app's own namespace (`<ns>::…`). Any other is refused here, before it reaches the host, with `IiiError` `forbidden`. `timeoutMs` defaults to 30 000, at most 120 000. |
| `iii.on(type, config, handler)` | Binds a trigger type of the app's namespace (another throws `forbidden` at once). Returns an unsubscribe function with a `ready` promise: bind first, read initial state after `ready`. |
| `iii.nav` | `push`, `pop`, `replace`, `popToRoot` over the pages of `defineApp`. |
| `iii.env`, `iii.onEnv` | `windowClass` (`compact`/`wide`, the 720 dp cut), `posture`, `platform`, `visible`, `theme`, `locale`, `online`; reactive in Solid scopes. Use `visible` to stop work nobody sees. |
| `iii.ui` | `toast`, `actionMenu` (an action sheet on the phone, an NSMenu on the Mac; resolves with the index or -1), `confirm`, `prompt`, `focus`. |
| `iii.clipboard.writeText`, `iii.openUrl` (asks the person; http, https, mailto), `iii.share`, `iii.saveFile` | |
| `iii.storage` | Key/value on the device per (environment, worker): keys up to 128 characters, values up to 64 KiB, 256 KiB in all. |
| `iii.reportError(err, { fatal? })` | Not fatal: the log. Fatal: the app's failure screen with Restart. |

A failed request rejects with `IiiError` (`code` is one of the catalogue's
`errorCodes` or the engine's own code).

### Hooks

- `useCall(fn, payload?, { enabled?, timeoutMs? })`: a Solid resource that
  calls again when `payload()` changes; `payload()` returning `undefined`
  means "do not call now".
- `useTrigger(type, config, handler, { enabled? })`: binds while the scope
  lives and `enabled()` holds; `config()` returning `null` unbinds.
- `useEnv()`, `useNav()`, `usePage()`.

The usual Solid primitives (`createSignal`, `createMemo`, `createEffect`,
`Show`, `For`, `Index`, `Match`, `onMount`, `onCleanup`, …) are re-exported,
so an app needs no direct `solid-js` dependency. `Switch` is the toggle
component; Solid's control-flow `Switch` comes from `solid-js`.

### What the VM has

ECMAScript itself, `console`, `setTimeout`/`setInterval`/`clear*`,
`queueMicrotask` and `performance.now()`. No DOM, `fetch`, `URL`,
`TextEncoder`, `atob`/`btoa`, `Intl`, `eval` or `new Function`.
`parseUrl(url)` stands in for `URL`. The package's types declare exactly
these globals (with `tsconfig.app.json`, which loads no DOM).

## Build

```js
import { buildNativeUi } from '@iii-dev/native-ui/build'

await buildNativeUi({ entry: 'src/app.tsx', outfile: 'dist/native.js', ns: 'browser', watch: process.argv.includes('--watch') })
```

esbuild bundles to one minified IIFE (ES2020, platform neutral, `browser`
and `production` conditions); `.tsx`/`.jsx` go through `babel-preset-solid`
in universal mode against `@iii-dev/native-ui/renderer` and
`@babel/preset-typescript`. `ns` is compiled in, so `iii.call` knows the
namespace before the host's `init`. Then:

- **size**: a warning above 128 KiB, an error above 256 KiB
  (`limits.bundleWarnBytes`, `limits.bundleFailBytes`);
- **smoke run** in the test harness: the bundle evaluates without a DOM,
  sends `ready` with a `root` page within the load budget, and mounts
  `root` without an error;
- the file is written only then, atomically, so the worker's hot-reload
  poller never serves a broken or half-written bundle.

It resolves with `{ outfile, bytes, sha256, warnings, ready }`. `watch:
true` rebuilds on every change and logs each result; a failed rebuild
leaves the last good bundle in place.

## Test harness

```ts
import { createTestHost } from '@iii-dev/native-ui/testing'

const host = await createTestHost({
  bundle: readFileSync('dist/native.js', 'utf8'),
  ns: 'browser',
  calls: { 'browser::sessions::list': () => ({ sessions: [] }) },
})
await host.mount('root')
await host.press('new-tab')            // a testTag, a node id or { type, ...props }
await host.type('address', 'example.com')
await host.itemPress('tabs', 's1')
await host.emitTrigger('browser::session-started', { session_id: 's1' })
await host.tick(150)                   // the fake clock: timers and microtasks
expect(host.tree()).toMatchInlineSnapshot()
```

The bundle runs in `quickjs-emscripten` with the device VM's globals (timers
on a fake clock, `console`, `performance.now`, no `eval`), its heap cap and
per-step deadline, turn by turn like the app's loop (deliver, timers,
microtasks). The fake host keeps the tree, answers requests (`calls`, `ui`
answers, `storage`), keeps the subscriptions and the deck, and checks the
protocol on every message: creates before use, inserts of detached nodes,
indexes in range, no node left detached at the end of a batch, bindings only
in templates, prop values of their catalogue type, messages within
`limits.maxMessageBytes`. Any violation or uncaught VM error fails the
action (`strict: false` to only record them).

API: `mount`, `unmount`, `back`, `tick`, `settle`, `deliver`, `respond`,
`fail`, `env`, `emitTrigger`, `press`, `longPress`, `emit`, `type`,
`typeWhileBusy`, `submit`, `toggle`, `select`, `itemPress`,
`itemLongPress`, `itemEmit`, `find`, `findAll`, `prop`, `tree`, `memory`,
`dispose`; and the records `messages`, `ops`, `toVm`, `transcript`,
`requests`, `pending`, `logs`, `errors`, `exceptions`, `violations`,
`toasts`, `menus`, `dialogs`, `clipboard`, `shared`, `saved`, `opened`,
`focusRequests`, `subscriptions`, `storage`, `deck`, `droppedWrites`.
Types: `src/testing/index.d.mts`.

## Conformance files

`conformance/*.json` are flows the harness produced from
`conformance/apps/*.tsx`, for the hosts to replay in their own unit tests:
`toVm` and `fromVm` hold the exact json-v0 messages of each direction,
`order` interleaves them, `tree` is the host's tree of every card at the
end, `host` what else the host did (writes it dropped, menus). Every string
of design §5.7 comes out of a flow byte for byte;
`conformance/design-examples.json` lists them with their decoded form.

| File | Flow |
|---|---|
| `counter.json` | §5.7: the counter, init + mount, a press |
| `list.json` | §5.7: a List with a bound ListRow template, an item press, a removal |
| `list-splice.json` | Two templates by `kind`, header/empty/footer slots, four `set` diffs, item presses |
| `text.json` | §5.7: two edits in one turn, the script's normalized write |
| `text-handshake.json` | A write with an old edit count dropped by the host, the person's text winning |
| `requests.json` | §5.7: a call and a binding, their answers, a trigger, a forbidden error |
| `nav-back.json` | §5.7: push, mount, Back with `{result: null}` |
| `nav-result.json` | A page popping itself with a result |
| `env.json` | §5.7: the window passing 720 dp |
| `action-menu.json` | An anchored action menu and the host's answer |

## The wire, as this SDK writes it

- VM → host: the SDK hands `__host_send` the **array** of entries (D16): the
  device's VM encodes it as `bin-v1`, the harness as json-v0. A batch the
  host refuses as too large (`RangeError`) is sent in halves; one entry that
  cannot fit is a fatal error.
- One batch per VM turn: the first op of a turn starts a short chain of
  microtasks, so promise continuations of the same turn (a response, then
  the app's `await`) land in the same message.
- Nodes reach the host lazily: a subtree goes out when it is attached to a
  node the host has (or becomes a page root), in one canonical walk
  (`create`, the type's props by tag, the common props by tag, then each
  slot's children, each then `insert`ed). Props at their default are not
  sent. A remove and re-insert in the same slot (how Solid moves some
  nodes) goes out as one `move`.
- At the end of a batch, every node the host has that ended detached gets
  `destroy`, after every other op. A node the app keeps and attaches again
  later goes out again under a new id.
- Host → VM stays json-v0 text into `__vm_receive`.

## Working on the package

```bash
pnpm -F @iii-dev/native-ui test          # vitest: units, harness, runtime, build, conformance, example
pnpm -F @iii-dev/native-ui typecheck     # the VM code (no DOM, no Node) and the Node code
node packages/native-ui/scripts/gen-catalog.mjs       # after a catalog.json change (orchestrated, with the ide copy)
node packages/native-ui/scripts/gen-conformance.mjs   # after a change that moves bytes on the wire
```

A test fails when `src/generated/` or `conformance/` is stale.
