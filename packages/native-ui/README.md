# @iii-dev/native-ui

The SDK for a worker's **native app** in the iii app. A worker ships a small
TSX bundle; the iii app runs it in a QuickJS VM on the device (one VM per
worker, its own thread, a heap cap and a CPU deadline, no `eval`, no
network) and draws what it describes with its own Jetpack Compose (Android)
and SwiftUI (Apple) components. The script decides **what** appears; the app
decides **how and when** (layout, virtualized lists, gestures, text editing,
phone/tablet/Mac arrangement).

Design and wire contract: `ide/projects/worker-native-ui/design.md` (§4 the
capability object, §5 the protocol, §6 the catalogue, §10 this package);
the platform as the app builds it (the VM, what the host decides, the
`RemoteScreen`): `ide/docs/worker-native-ui.md`. The machine schema is
[`catalog.json`](catalog.json); the ide repo keeps a byte-identical copy at
`native/catalog/catalog.json`, checked by a unit test there. How a worker
ships an app end to end: [`docs/sops/native-ui.md`](../../docs/sops/native-ui.md).

**Linked by `workspace:*`, never published.**

## What is in the package

| Entry | What |
|---|---|
| `@iii-dev/native-ui` | `defineApp`, the components (`Column` … `RemoteScreen`), `createList`, `List` and `itemHandler`, the `iii` capability object, the hooks, `parseUrl`, `IiiError`, and the Solid primitives an app needs |
| `@iii-dev/native-ui/renderer` | The `solid-js/universal` renderer the JSX compiles against (the build wires it; apps never import it) |
| `@iii-dev/native-ui/build` | `buildNativeUi`: TSX → one IIFE bundle, with size limits, a smoke run and `--watch` |
| `@iii-dev/native-ui/testing` | `createTestHost`: the bundle in `quickjs-emscripten` with the device VM's globals, driven like the app drives it |
| `@iii-dev/native-ui/catalog.json` | The canonical schema |
| `@iii-dev/native-ui/tsconfig.app.json` | The compiler options an app extends (no DOM, no Node, JSX for Solid) |
| `@iii-dev/native-ui/tsconfig.test.json` | The same for the app's tests, which run in Node (ES2023, Node types) |

## An app

```
<worker>/native-ui/
  package.json        dependencies: "@iii-dev/native-ui": "workspace:*"
                      devDependencies: "solid-js": "catalog:" (the JSX types), "@types/node", "typescript", "vitest"
  tsconfig.json       { "extends": "@iii-dev/native-ui/tsconfig.app.json", "include": ["src"] }
  tsconfig.test.json  { "extends": "@iii-dev/native-ui/tsconfig.test.json", "include": ["test", "vitest.config.mjs"] }
  build.mjs           buildNativeUi({ entry: 'src/app.tsx', outfile: 'dist/native.js', ns: '<ns>' })
  src/app.tsx         defineApp({ pages: { root: … } })
```

The full template, and why `solid-js` must be listed under pnpm, is in the
SOP.

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
writes `examples/hello/dist/native.js`. [`examples/catalog2`](examples/catalog2)
does the same for catalogue 2 (`build:catalog2`, `dist/catalog2.js`), one
page per new piece, with the testTags a host's instrumented tests use.

### Pages

`defineApp({ pages, catalog? })` declares every page once; `root` is required. A page
is a component `(props: { page, params }) => JSX` or `{ component, modal?,
fullScreen? }`. The host mounts and unmounts page instances (one per card
of the app's deck); `iii.nav.push(name, params)` stacks a page and resolves
with the result its card leaves with (`usePage().pop(result)`), or `null`
after Back. `usePage()` gives the instance (`inst`, `name`, `params`) with
`pop` and `replace` bound to it.

### The catalogue an app needs

`catalog` (default 1) is the lowest catalogue version the app needs; it goes
in `ready`, and an iii app with an older catalogue shows "Needs a newer iii
app" instead of running it. Declare `catalog: 2` to use what catalogue 2
added (`Markdown`, `Lanes`, `hue`, `List.dragGroup`/`onDrop`,
`ListDetail.listFill`), and give the worker's descriptor the same
(`NativeUi::catalog(2)`, which the hub reads before it loads anything). The
SDK warns once when an app uses a piece newer than it declared (the
generated `SINCE` table says which version added what); an app that needs
none of it keeps 1, as the browser does, and runs on older apps too.

```tsx
defineApp({ catalog: 2, pages: { root: Board, ticket: Ticket } })
```

### Components

One component per catalogue type, with the catalogue's names: `Column`,
`Row`, `Box`, `Scroll`, `Spacer`, `Divider`, `Text`, `Icon`, `Badge`,
`StatusDot`, `Spinner`, `Progress`, `EmptyState`, `Image`, `StatusPanel`,
`Markdown`, `Button`, `IconButton`, `TextField`, `Switch`, `Segmented`,
`Chip`, `List`, `ListRow`, `Card`, `Lanes`, `Page`, `ListDetail`, `Sheet`,
`RemoteScreen`.

- **Props** are typed from the catalogue (`src/generated/props.ts`). Enums
  take their names (`role="title"`, `kind="primary"`); `null`/`undefined`
  means the default. Every component takes the common props `visible`,
  `testTag`, `a11yLabel`, `weight`, `width`, `height` (-1 = fill),
  `padding` (a number, `[v, h]` or `[top, end, bottom, start]`),
  `alignSelf`, and `ref` (a `NodeRef`, the anchor of `iii.ui.actionMenu`
  and the target of `iii.ui.focus`).
- **Events** are `on<Event>` props (`onPress`, `onLongPress`, `onChange`,
  `onSubmit`, `onSelect`, `onFocus`, `onDismiss`, `onRefresh`,
  `onEndReached`, `onFrame`, `onError`, `onKeyboard`, `onReady`, `onLink`,
  `onDrop`). A handler
  subscribes the node; the host only sends subscribed events and only arms
  the gestures behind them (no long-press haptic without `onLongPress`).
- **Slots**: `children` is slot 0; the other slots are props named after
  them (`actions`, `top`, `bottom`, `leading`, `trailing`, `list`, `detail`,
  `emptyDetail`, `header`, `empty`, `footer`). Where slot 0 has another name
  (`EmptyState.actions`, `StatusPanel.action`, `Page.body`,
  `Sheet.content`, `Lanes.lanes`), that prop works too.
- **Text**: the JSX children of `Text`, `Button`, `Badge`, `Chip` and
  `Markdown` become their `text` (`<Text>Count {n()}</Text>`). Text
  anywhere else is not shown (a warning in the log).
- `Switch`, `Segmented` and `Lanes` show the person's change at once; if the
  app does not take it (the `checked`/`selected` prop stays), the SDK sends
  the app's value back.
- `RemoteScreen` is only configured from the script (`session`, `wire`,
  `enabled`, `mode`, `mobile`, `keyboard`, `placeholder`, `maxScale`):
  frames and gestures go between the worker and the native view and never
  through the VM. Every function and the trigger in its `wire` must be in
  the app's namespace, or the view stays empty (the host logs why). Its
  events: `onReady()`, `onFrame(width, height, modeChanged)` (reload the
  page once when `modeChanged`), `onLongPress(x, y)` in page coordinates,
  `onKeyboard(false)` when the person closes the keyboard, `onError(message)`
  when the stream fails. The wire of the browser and the placeholders:
  design §6.7 and §6.8, `catalog.json` › `remoteScreen`.

### Catalogue 2: Markdown, Lanes, drag and drop, hues, listFill

All of these need `defineApp({ catalog: 2, … })`.

**`Markdown`** draws CommonMark (GFM tables, strikethrough, autolinks) with
the app's chat renderer; `dense` is the small type of a comment or a card,
`selectable` lets the person select text. Images show as their alt text and
raw HTML as text; past `limits.maxMarkdownChars` (200 000) the host draws
the start and a note. The host **never opens a link**: a tap is
`onLink(href)`, with the destination as written, and the script decides.

```tsx
<Markdown dense onLink={(href) => (href.startsWith('kan:') ? openTicket(href.slice(4)) : iii.openUrl(href))}>
  {ticket().description}
</Markdown>
```

**`Lanes`** shows its children, one per title, as columns side by side when
at least two fit the host's width (`laneWidth`, default 300 dp), or one per
page under a tab strip. `counts` puts a short text after each title.
`selected` is the lane in view (a page) or the one scrolled into view
(columns); `onSelect(index)` comes when the person settles on another page,
optimistic like `Segmented`'s. Each lane keeps its own scroll.

```tsx
<Lanes titles={['Todo', 'Doing', 'Done']} counts={lanes.map((l) => String(l.items().length))}
  selected={lane()} onSelect={setLane}>
  {lanes.map((source) => <List source={source} templates={[card]} />)}
</Lanes>
```

**Drag and drop**: a `List` with `dragGroup` lets a long press lift an item;
released without moving it is the item's `onLongPress`, moved it is a drag
to any `List` of the same group (itself included: a reorder). Releasing it
over a List fires that List's `onDrop(key, index, source)`: `key` is the
item's key, `index` its position among the target's items once it is out of
its old place, and `source` the List it came from as a `NodeRef`, the same
object that List's `ref` gave (null if it is gone), so compare it with your
refs:

```tsx
const refs: (NodeRef | undefined)[] = []
const drop = (to: number) => (key: string | number, index: number, source: NodeRef | null) => {
  const from = source ? refs.indexOf(source) : -1
  const item = from < 0 ? undefined : lanes[from].get(key)
  if (!item) return
  batch(() => {
    lanes[from].remove(key)
    lanes[to].splice(index, 0, { ...item, status: STATUSES[to] })
  })
  iii.call('kanban::ticket::move', { id: key, status: STATUSES[to] }).catch(() => {
    // put it back, and say so
  })
}
{lanes.map((source, i) => (
  <List ref={(n) => (refs[i] = n)} source={source} dragGroup="cards" onDrop={drop(i)} templates={[card]} />
))}
```

**Hues**: `hue` on `Icon`, `Badge`, `StatusDot` and `Chip` (`none`, `blue`,
`purple`, `teal`, `green`, `amber`, `rose`: the app's agent and project
colours) replaces the tone, or the Badge's variant, when it is not `none`.
In a List template, bind it to an item field holding the name
(`hue={row.hue}` with `{ hue: 'teal' }`): see Lists below.

**`ListDetail.listFill`**: on a wide window, while `showDetail` is false the
list takes the whole width (no detail column, no `emptyDetail`, no hide
button), so a board of `Lanes` fills the screen; with `showDetail` the list
goes back to its usual width beside the detail. No effect on a compact
window. Open a card by setting the lane and `showDetail` in one batch, so
the narrowed board shows the card's lane:

```tsx
const open = (lane: number) => (item: Ticket) => batch(() => { setLane(lane); setOpen(item.id) })
```

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
  compute the strings a row shows into the item. A bound field holds what
  the prop takes (a string for `text`, a boolean for `visible`); the harness
  fails an item whose bound field has the wrong type.
- An enum prop bound to a field (`hue={row.hue}`, `tone={row.tone}`) takes
  the enum's name in the item, as a plain prop does: the host reads an
  index, so the SDK sends the index (an unknown name as null, the default,
  with a warning). Your items keep the names; the harness's `items()` shows
  what the host got. Don't bind the same field to a text prop too.
- Handlers inside a template get the item first (found by the key of the
  host's `itemEvent`). Argument-less handlers are typed `(item?: any)`;
  inside a template, annotate the item (`(item: Tab) => …`). A handler
  whose event has arguments (a Markdown's `onLink`, a Switch's `onChange`)
  is typed for those arguments only; wrap it in `itemHandler` to take the
  item first, typed:
  `<Markdown dense text={row.body} onLink={itemHandler((c: Comment, href: string) => open(c, href))} />`.
  The harness fires it with `itemEmit(list, key, node, 'link', [href])`.
- `createList({ key, kind?, items? })`: `items()` (reactive), `set(all)`
  (only what changed travels: the changed positions when keys keep their
  order, else one splice between the common head and tail; an item that is
  the same object as before is skipped without a comparison, so keep the
  unchanged ones when you can), `splice`,
  `update(key, patch)`, `remove(key)`, `get(key)`. Treat items as immutable
  JSON; change them through the source.
- Keys must be unique: the host logs a duplicate (and suffixes it), the
  harness fails on it, and `set` warns once. When worker data may repeat a
  key, `lastPerKey(items, 'key')` keeps the last entry of each key, where it
  last appears.
- `TextField`, `List`, `RemoteScreen` and `Sheet` cannot sit in a template:
  the host draws a neutral placeholder there. Neither the SDK nor the
  harness checks it.

### The capability object

Everything outside the tree goes through `iii` (design §4.2), as a request
the host checks:

| | |
|---|---|
| `iii.app` | `{ key: 'worker:<ns>', ns }` |
| `iii.call(fn, payload?, { timeoutMs? })` | A function of the app's own namespace (`<ns>::…`; never `iii::…` or `engine::…`). Any other is refused here, before it reaches the host, with `IiiError` `forbidden`; the host checks again. `timeoutMs` defaults to 30 000, at most 120 000. |
| `iii.on(type, config, handler)` | Binds a trigger type of the app's namespace, `<ns>::…` or `<ns>:…` (`kanban:change`); another throws `forbidden` at once (never `iii::…` or `engine::…`). Returns an unsubscribe function with a `ready` promise: bind first, read initial state after `ready`. |
| `iii.nav` | `push`, `pop`, `replace`, `popToRoot` over the pages of `defineApp`. |
| `iii.env`, `iii.onEnv` | `windowClass` (`compact`/`wide`, the 720 dp cut), `posture`, `platform`, `visible`, `theme`, `locale`, `online`; reactive in Solid scopes. Use `visible` to stop work nobody sees. |
| `iii.ui` | `toast`, `actionMenu` (an action sheet on the phone, an NSMenu on the Mac; resolves with the index or -1), `confirm`, `prompt`, `focus`. |
| `iii.clipboard.writeText`, `iii.openUrl`, `iii.share`, `iii.saveFile` | `openUrl` always asks the person, with the address in view, and takes only http, https and mailto (another scheme rejects here with `forbidden`); `share` and `saveFile` take up to 25 MiB. |
| `iii.storage` | `get`, `set`, `remove`, `keys`: key/value on the device per (environment, worker); keys up to 128 characters, values up to 64 KiB of JSON, 256 KiB in all (`quota`). |
| `iii.reportError(err, { fatal? })` | Not fatal: the log. Fatal: the app's failure screen with Restart. |

A failed request rejects with `IiiError` (`code` is one of the catalogue's
`errorCodes` or the engine's own code).

### Hooks

- `useCall(fn, payload?, { enabled?, timeoutMs? })`: returns
  `[resource, { refetch, mutate }]`, a Solid resource that calls again when
  `payload()` changes; `payload()` returning `undefined` means "do not call
  now"; without `payload` it calls once.
- `useTrigger(type, config, handler, { enabled? })`: binds while the scope
  lives and `enabled()` holds; `config()` returning `null` unbinds.
- `useEnv()`, `useNav()`, `usePage()`.

The usual Solid primitives (`createSignal`, `createMemo`, `createEffect`,
`createSelector`, `Show`, `For`, `Index`, `Match`, `onMount`, `onCleanup`,
…), its types (`Accessor`, `Setter`, `Signal`, `Component`,
`ParentComponent`, `ParentProps`, `Resource`, `JSX`, …) and the store
(`createStore`, `produce`, `reconcile`) are re-exported from the bundle's one
copy of solid-js. `Switch` is the toggle component; Solid's control-flow
`Switch` comes from `solid-js`.

JSX is typed for the VM: the package declares the `Node` that Solid's
`JSX.Element` refers to (without the DOM it would be `any`), so a plain
object is not an element and `<Show when={tab()}>{(t) => …}</Show>` gets a
typed `t`.

### What the VM has

ECMAScript itself, `console`, `setTimeout`/`setInterval`/`clear*`,
`queueMicrotask` and `performance.now()`. No DOM, `fetch`, `URL`,
`TextEncoder`, `atob`/`btoa`, `Intl`, `eval` or `new Function`.
`parseUrl(url)` stands in for `URL`. The package's types declare exactly
these globals (with `tsconfig.app.json`, which loads no DOM).

### Pitfalls

What has broken apps on a device, and passes a quick test unless you look
for it:

- **No regex from user text in the VM.** A pattern a person typed (a log
  filter) can backtrack catastrophically; the script then passes the
  250 ms step deadline and the whole app stops with "stopped responding".
  Filter in the script with a plain substring (`text.toLowerCase().includes(q)`),
  and send a regex only to the worker, which runs it outside the VM.
- **Check worker data entry by entry.** A list from the worker can hold one
  bad entry (a missing field, a number where a string was due): skip that
  entry, don't throw and lose the whole list. Read fields with type checks,
  never trust the shape.
- **Every call can fail or hang.** Pass `timeoutMs` (or rely on the 30 s
  default; the host enforces it, and so does the test harness), and make
  the UI state behind a call recover when it rejects: a "loading" flag, a
  disabled button, a spinner must clear in a `finally`, or the screen stays
  stuck after one hung call.
- **Gate what reaches navigation-like calls.** Anything the person types or
  the worker sends that ends up opening something (a URL to navigate, a
  path, a command) passes one gate function first; the browser's `webUrl`
  (`browser/native-ui/src/address.ts`) lets only well-formed http(s) URLs
  through, so `javascript:`, `data:`, `file:` and the like never reach the
  worker. `iii.openUrl` already allows only http, https and mailto.
- **Markdown never opens a link itself.** A tap is `onLink(href)` and
  nothing else; without `onLink`, links do nothing. The href is whatever
  the text wrote (worker data, so `javascript:` too): route your own scheme
  (`kan:KAN-12`) to `iii.nav.push`, and hand the rest to `iii.openUrl`,
  which takes only http, https and mailto and asks the person first.
- **A drop is optimistic; the script must splice.** The host keeps showing
  the item where it was dropped only until the next batch that changes
  either List, or for 1 s; then the lists show what the script left. Splice
  both lists in `onDrop` itself (one `batch`), then call the worker, and on
  a failure put the item back and toast. A handler that only calls the
  worker makes the card jump back and forth.
- **Lanes leaves the layout to the host.** Don't pick columns or pages from
  `iii.env.windowClass`, and don't build your own tab strip: the host
  decides from its own width (a ListDetail's list column is narrower than
  the window) and `laneWidth`. In columns `onSelect` does not come; keep
  `selected` for the lane you want in view (the card you opened).

## Build

```js
import { buildNativeUi } from '@iii-dev/native-ui/build'

await buildNativeUi({ entry: 'src/app.tsx', outfile: 'dist/native.js', ns: 'browser', watch: process.argv.includes('--watch') })
```

Options: `entry`, `outfile`, `ns` (required; it must match
`^[a-z0-9][a-z0-9_-]{0,63}$`, exported as `NS_PATTERN`), `minify` (default
`true`), `smoke` (default `true`), `watch` (default `false`), `log` (default
`console`). esbuild bundles to one IIFE (ES2020, platform neutral, `browser`
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

It resolves with `{ outfile, bytes, sha256, warnings, ready }`. With
`watch: true` it resolves with `{ stop() }` instead, rebuilds on every
change and logs each result; a failed rebuild leaves the last good bundle in
place. The same module exports `checkBundle(source, { ns, smoke })` (the
size and smoke checks on a bundle you already have) and `nativeUiPlugin()`
(the esbuild plugin, for a build of your own).

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

Options, with their defaults: `bundle` (required), `ns` (`demo`), `appKey`
(`worker:<ns>`), `catalog` (the catalogue the host renders, `init.catalog`:
the SDK's, 2; a bundle whose `ready` needs more fails to load with
`VmFailure` kind `catalog`, as the app shows "Needs a newer iii app"),
`host` and `engine` (what `init` says: `android 0.1.0`,
`quickjs-emscripten 0.31.0`), `env` (merged over `DEFAULT_ENV`), `calls`,
`ui` (the dialogs' answers; without one, `actionMenu` −1, `confirm` false,
`prompt` null, `openUrl` true), `storage`, `now` (the fake clock's start,
0), `rawRequests`, `ids` (seeds the SDK's node, request and subscription
counters), `strict` (true), `autoInit` (send `init` with the first action,
true), `memoryLimit` (64 MiB), `stackSize` (1 MiB), `stepBudgetMs` (250),
`loadBudgetMs` (2000), `maxMessageBytes` (8 MiB), `log` (every console line).
The two deadlines are wall time, like the device's; without the options, the
catalogue's values are multiplied by `III_NATIVE_UI_BUDGET_SCALE` (1 when unset;
CI sets 4, since a shared runner is several times slower and noisier).
A `call` whose function is not in `calls` answers `function_not_found`; a
`calls` handler that throws answers its `code` (throw a `HostError(code,
message)`), or `invocation_failed`. Like the device host, a call that has
not answered when the fake clock passes its `timeoutMs` (default
`limits.callTimeoutMsDefault`, 30 000; at most 120 000) fails with
`{code: "timeout", message: "The engine didn't answer in time"}`, and a
later answer is dropped: a handler returning a promise that never settles
plus `tick(30_000)` is how a test shows the UI recovering from a hung call.

The bundle runs in `quickjs-emscripten` with the device VM's globals (timers
on a fake clock, `console`, `performance.now`, no `eval`), its heap cap and
per-step deadline, turn by turn like the app's loop (deliver, timers,
microtasks). The fake host keeps the tree, answers requests (`calls`, `ui`
answers, `storage`), keeps the subscriptions and the deck, and checks the
protocol on every message: creates before use, inserts of detached nodes,
indexes in range, no node left detached at the end of a batch, bindings only
in templates, prop values of their catalogue type (and bound item fields
too), messages within `limits.maxMessageBytes`. Its policy is the host's:
calls to `<ns>::…`, bindings of `<ns>::…` and `<ns>:…`. Any violation or uncaught VM error fails the
action (`strict: false` to only record them). Like the device, a step
that passes its deadline fails with `watchdog` even when the interrupt
landed in a promise job (QuickJS turns it into a rejection there), and
running out of memory in a timer or a microtask fails with `memory`. `rawRequests: true` exposes a
test-only `__iiiui_request(method, args)` in the VM that skips the SDK's own
checks, to show what the host answers to a request the SDK would refuse.

API: `mount` (resolves with the page instance), `unmount`, `back`, `tick`,
`settle`, `deliver`, `respond`, `fail`, `env`, `emitTrigger` (resolves with
how many subscriptions got it), `press`, `longPress`, `emit`, `type`,
`typeWhileBusy`, `submit`, `toggle`, `select`, `link`, `drop`,
`itemPress`, `itemLongPress`, `itemEmit`, `find`, `findAll`, `prop`,
`items`, `keys`, `lanes`, `tree`, `memory`, `dispose`; and the records `messages`, `ops`, `toVm`, `transcript`,
`requests`, `pending`, `logs`, `errors`, `exceptions`, `violations`,
`toasts`, `menus`, `dialogs`, `clipboard`, `shared`, `saved`, `opened`,
`focusRequests`, `subscriptions`, `storage`, `deck`, `droppedWrites`; the
state `ready` (the bundle's `ready` info), `roots`, `failed` (a `VmFailure`'s
kind and message once the VM stopped: `watchdog`, `memory`, `reported`,
`load`, `protocol`, `catalog`), `envState`, `clock`, `initInfo()`.

The catalogue 2 events, as the device host sends them:

```ts
await host.link('description', 'kan:KAN-12')      // a Markdown: link [href]; the href must be in its text
await host.select('board', 2)                      // a Lanes (or a Segmented): shown at once, then select [index]
await host.drop('lane-doing', 'KAN-3', 0, 'lane-todo') // drop [key, index, source] on the target List
await host.drop('lane-todo', 'KAN-1', 2)           // a reorder: the source defaults to the target
await host.itemLongPress('lane-todo', 'KAN-1')     // a lifted card released without moving
host.keys('lane-doing')                            // ['KAN-3', …]: the List's keys, in order
host.items('lane-doing')                           // its items as the host holds them (enum fields as indexes)
host.lanes('board')                                // { titles, counts, selected, lanes: NodeView[] }
```

`drop` checks what the host guarantees: both Lists share a `dragGroup`, the
key is in the source, the index is in range (`0..count`, one less for a
reorder), and the target subscribed `onDrop`. Like the device, the lists
then show what the script left. The module also
exports `HostError`, `VmFailure`, `DEFAULT_ENV`, `createDecoder`,
`decodeMessage` (a json-v0 message of either direction as named entries, for assertions) and `HostTree`.
Types: `src/testing/index.d.mts`.

## Conformance files

`conformance/*.json` are flows the harness produced from
`conformance/apps/*.tsx`, for the hosts to replay in their own unit tests.
A flow has `name`, `description`, `catalog` (the catalogue its bundle
declares), `encoding` (`json-v0`), `app`,
`ns`; `toVm` and `fromVm` hold the exact json-v0 messages of each
direction, `order` interleaves them, `tree` is the host's tree of every card
at the end, `host` what else the host did (`droppedWrites`, `toasts`,
`menus`, `errors`), and `designExamples` the design strings it produced.
Every string of design §5.7 comes out of a flow byte for byte;
`conformance/design-examples.json` lists them (`id`, `dir`, `message`,
`decoded`, `flow`). Those flows run on a catalogue 1 host (their `init`
says `"catalog":1`, as the design writes it); the catalogue 2 flows run on
the SDK's.

The ide repo copies these files into
`android/app/src/test/resources/workerui/conformance/`, replays each flow
through the Android host, and turns every `fromVm` into the binary form the
device's VM writes (`bin/*.bin`, written by `native/iiiui/test`'s
`iiiui_test --convert`): a change here is a change there, in the same
piece of work (`ide/native/README.md` › Conformance fixtures).

| File | Flow |
|---|---|
| `counter.json` | §5.7: the counter, init + mount, a press |
| `list.json` | §5.7: a List with a bound ListRow template, an item press, a removal |
| `list-splice.json` | Two templates by `kind`, header/empty/footer slots, four `set` diffs, item presses |
| `text.json` | §5.7: two edits in one turn, the script's normalized write |
| `text-handshake.json` | A write with an old edit count dropped by the host, the person's text winning |
| `requests.json` | §5.7: a call and a binding, their answers, a trigger, a call outside the namespace refused by the host |
| `nav-back.json` | §5.7: push, mount, Back with `{result: null}` |
| `nav-result.json` | A page popping itself with a result |
| `env.json` | §5.7: the window passing 720 dp |
| `action-menu.json` | An anchored action menu and the host's answer |
| `markdown.json` | Catalogue 2: a dense, selectable Markdown; two link taps (an app link, a web link) |
| `lanes-select.json` | Catalogue 2: a board (ListDetail `listFill` > Lanes > three Lists with `dragGroup`, cards with a bound `hue`); select on two lanes |
| `list-drop.json` | Catalogue 2: a reorder, a drop across lanes, a drop into an empty lane, each spliced by the script |
| `list-fill.json` | Catalogue 2: a card opened (`selected` and `showDetail` in one message), then dismissed |
| `hue.json` | Catalogue 2: Icon, Badge and StatusDot in every hue, a Chip whose hue changes |

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
pnpm -F @iii-dev/native-ui test          # vitest: units, catalog, harness, runtime, build, conformance, example
pnpm -F @iii-dev/native-ui typecheck     # the VM code (no DOM, no Node) and the Node code
pnpm -F @iii-dev/native-ui gen           # scripts/gen-catalog.mjs: src/generated/ after a catalog.json change
node packages/native-ui/scripts/gen-conformance.mjs   # after a change that moves bytes on the wire
pnpm -F @iii-dev/native-ui build:example # examples/hello/dist/native.js (also takes --watch)
pnpm -F @iii-dev/native-ui build:catalog2 # examples/catalog2/dist/catalog2.js
```

Both generators take `--check` (exit 1 when their output is stale), and a
test (`test/catalog.test.ts`, `test/conformance.test.ts`) fails when
`src/generated/` or `conformance/` is stale. `examples/hello` and
`examples/catalog2` are tested by `test/example.test.ts`. A `catalog.json`
change is orchestrated with the ide copy (`native/catalog/catalog.json`)
and its generated Kotlin. A new catalogue version also lists what it added
in `ADDED` of `scripts/gen-catalog.mjs` (catalog.json keeps no history),
which the generator checks against catalog.json and turns into `SINCE`.

In CI, the `native-ui` job (`.github/workflows/ci.yml`) runs biome, this
package's typecheck and tests, the generators' `--check`, and every
`<worker>/native-ui` app's `typecheck` and `test`, whenever the SDK, an app,
`crates/native-ui/`, the root lockfile or the workspace file changes; and a
change here puts every worker whose app depends on `@iii-dev/native-ui` in
the `rust` matrix (`discover_changed_workers.py`, `BUNDLED_PACKAGES`), so
its embedded bundle is rebuilt and tested. The SOP's §6 has the local
equivalents.
