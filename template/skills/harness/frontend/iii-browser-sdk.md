---
title: iii-browser-sdk
description: Make a browser app a first-class iii worker with iii-browser-sdk — registerWorker, registerFunction, call, observe and the observer hooks, engine state, and streaming channels, wired into React and Vite.
type: how-to
---

# iii-browser-sdk

`iii-browser-sdk` turns the frontend into an iii worker: one WebSocket to the engine instead
of many HTTP round-trips, backend events reaching the UI through observers, and the same
primitives used server-side (`registerFunction`, `call`, `observe`). No Node.js
dependencies, no OpenTelemetry — it runs on native `WebSocket`.

Docs <https://iii.dev/docs/tutorials/linkly/frontend> ·
API reference <https://iii.dev/docs/reference/sdk-browser> ·
npm <https://www.npmjs.com/package/iii-browser-sdk>

**The installed types are the contract.** Before writing SDK code, read the project's
`node_modules/iii-browser-sdk/dist/index.d.mts` (plus `state.d.mts`, `helpers.d.mts`), or the
published file at `https://cdn.jsdelivr.net/npm/iii-browser-sdk@<version>/dist/index.d.mts`.
Never write these calls from memory: the export names and the `InitOptions` keys are the
exact surface, and the internal type chunk is content-hashed, so re-read it per version.

## Install, and the only entry point that matters

```bash
pnpm add iii-browser-sdk
```

```ts
import { registerWorker } from 'iii-browser-sdk'

const iii = registerWorker(import.meta.env.VITE_III_WS_URL) // ws://localhost:3111
```

`registerWorker(address, options?)` is the sole top-level function; everything else hangs off
the returned `ISdk` — `registerFunction`, `call`, `observe`, `onObserverAdded`,
`onObserverRemoved`, `onObserverMatch`, `onObserverDeliver`, `observers`, `isObserved`,
`currentInvocation`, `withInvocation`, `addConnectionStateListener`, `getFatalError`,
`shutdown`. Do not destructure these as top-level imports; they are not exported that way.
The package root also exports `CallAction`, `ObserveRejection`, `EngineSources` and
`EngineFunctions`.

`InitOptions`: `workerName`, `namespace`, `invocationTimeoutMs` (default 30000),
`reconnectionConfig`, `headers` (ignored — see auth). The browser has no `process.env`, so
`namespace` has no environment fallback: pass it explicitly when you need one.

## Registering a function the backend can call

```ts
const ref = iii.registerFunction(
  'ui::show-notification',
  async (data: { title: string; body: string }) => {
    toast.push(data.title, data.body)
    return { displayed: true }
  },
  { description: 'Shows a toast in the open tab' },
)
// later
ref.unregister()
```

- The handler is `async (payload) => result` and runs **in the browser tab**.
- Pass a `description` (plus `request_format` / `response_format` when callers need the
  contract) — that is what `engine::functions::info` shows.
- Return JSON-serializable data; a thrown error surfaces as the caller's error.
- A backend worker reaches it by id: `iii.call({ function_id: 'ui::show-notification', payload })`.
- The handler's return value is what the function's observers observe.

## Calling backend functions

```ts
const users = await iii.call<{ limit: number }, User[]>({
  function_id: 'api::get::users',
  payload: { limit: 20 },
  timeoutMs: 5000,
})
```

Three routing modes — choose deliberately:

| Mode | Call | Use when |
| --- | --- | --- |
| await (default) | `await iii.call({ function_id, payload })` | you need the result |
| fire-and-forget | `action: CallAction.Void()` | telemetry, non-essential side effects |
| enqueue | `action: CallAction.Enqueue({ queue })` | long work; resolves to `{ messageReceiptId }` |

`namespace` on the request routes the invocation elsewhere; omit it to stay in your own.
`metadata` rides beside the payload and reaches the handler as its second argument.

## Receiving live pushes (there is no polling story)

Every function is observable: when a backend function runs, its observers run. A tab registers
a handler and observes the backend's event function with it:

```ts
const tab = crypto.randomUUID().slice(0, 8)
const handlerId = `ui::orders-changed::${tab}`

const handler = iii.registerFunction(handlerId, async (event: OrderEvent) => {
  store.upsert(event.order)
  return null
})

const observer = iii.observe(
  {
    function_id: 'orders::on-change',
    filters: { 'input.customer_id': { $eq: customerId } },
    deliver: 'output', // the handler receives the event itself
  },
  handlerId,
)
// teardown
await observer.dispose()
handler.unregister()
```

That is the whole real-time design: the backend calls its event function after every change and
the open tab updates. Do not add polling, SSE, or a second socket alongside it.

- A function id has one host per namespace: a second tab that registers the same id is refused
  (`FUNCTION_NAMESPACE_CONFLICT`). A handler that one tab observes with carries a per-tab suffix.
- `observe` returns an `ObserverHandle` at once (`id`, `state`, `ready()`, `onStateChange`,
  `dispose()`); the observer is `pending` until the observed function's worker is connected,
  and the SDK observes it again after a reconnect.
- `filters` select calls by `input`, `output`, `error`, `caller` and `metadata`, each with an
  explicit operator (`$eq`, `$in`, `$gte`, …). `args` go to the worker that hosts the function,
  for a function whose `args_format` takes them.
- `deliver` picks what the handler receives: the `Observed` envelope (default), the call's
  `input`, or its `output`. An event function returns its event, so `'output'` is the event.
- Options set the observer's `id`, `metadata` (the handler's second argument), `max_fires`,
  `expires_in_ms` and `max_in_flight`.
- The engine's own events are functions too: `EngineSources.FUNCTIONS_CHANGED`
  (`engine::on-functions-changed`), `EngineSources.WORKERS_CHANGED` and the others.

A backend worker that must reach one known function in the tab calls it by id:
`iii.call({ function_id: 'ui::show-notification', payload })`.

## Hosting an observable function in the browser

The tab hosts a function others observe only when the browser itself is the event source. Its
worker decides what observing it means with four hooks, each overriding a default:

| Hook | Default |
| --- | --- |
| `onObserverAdded(obs)` | accept the observer; throw `ObserveRejection` to refuse it |
| `onObserverRemoved(obs)` | stop delivering to it and forget it |
| `onObserverMatch(obs, call)` | every observer wants every call |
| `onObserverDeliver(call, observers)` | run every matching observer whose filters match; return the handler's value |

```ts
import { CallAction, ObserveRejection } from 'iii-browser-sdk'

iii
  .registerFunction('ui::on-click', (click: { selector: string; x: number; y: number }) => click, {
    args_format: { type: 'object', required: ['selector'], properties: { selector: { type: 'string' } } },
    private: true, // only this tab calls it; anyone may observe it
  })
  .onObserverAdded((obs) => {
    const selector = obs.args.selector as string
    if (!document.querySelector(selector)) throw new ObserveRejection('NO_SUCH_ELEMENT', `nothing matches ${selector}`)
    clicks.retain(selector, (event) =>
      iii.call({
        function_id: 'ui::on-click',
        payload: { selector, x: event.clientX, y: event.clientY },
        action: CallAction.Void(),
      }),
    )
  })
  .onObserverRemoved((obs) => clicks.release(obs.args.selector as string))
  .onObserverMatch((obs, call) => obs.args.selector === call.input.selector)
```

With many observers, match by keys (`{ observer: (obs) => key, call: (call) => key }`): the SDK
keeps an index, one lookup per key. `iii.isObserved(id, { input })` says whether a call would
reach anyone, so a source skips building an event nobody receives. `onObserverDeliver` delivers
each call itself through `iii.call({ observer, payload })` and returns what the caller gets back;
a call through an observer belongs there. To send one observer its current state, call the
function with `metadata: { for_observer: obs.id }` once `obs.accepted()` resolves (catch the
rejection: a rejected observer gets nothing), and have the match hook give such a call to that
observer alone (`!call.metadata?.for_observer || call.metadata.for_observer === obs.id`); iii
delivers it like every call, so the target receives every event in one shape.
A function that sets `args_format`, `private`, `observable: false`, a match hook or a deliver
hook needs an engine with `observe.v2`; against an older one it stays unregistered and the SDK logs
`UNSUPPORTED` once.

## State, helpers, channels (subpath exports)

- `iii-browser-sdk/state` — the engine's state worker, typed: `get<T>`/`set`/`delete`/`list`/
  `update`, plus `StateEventType.Created|Updated|Deleted` for the change events
  `state::on-changed` is called with. Use `update` with ordered `ops` when you need an atomic edit instead of a
  read-modify-write race.
- `iii-browser-sdk/helpers` — `createChannel(iii, bufferSize?)` returns a `Channel` with
  `writer` / `reader` and *serializable* `writerRef` / `readerRef`, so a ref can be handed to
  a backend worker and the bytes stream either direction; `createStream(iii, name, impl)` wires
  a `stream::get/set/delete/list/list_groups` implementation; `isChannelRef` and
  `extractChannelRefs` (recursive, returns `[path, ref]` tuples) complete the set.
- `ChannelWriter.sendMessage` / `sendBinary` / `close`; `ChannelReader.onMessage` /
  `onBinary` / `readAll`.
- Some releases publish `iii-browser-sdk/stream` as an empty module with types only. Read the
  installed `dist/` for the version in front of you instead of assuming.

## Connection, auth, and the failure modes that cost an afternoon

- **Auth is RBAC at the listener, not headers.** A browser cannot set headers on a WebSocket
  handshake, so `headers` is ignored: credentials ride as **query parameters or cookies** and
  are judged by the RBAC *auth function* on the upgrade request (`{ headers, query_params,
  ip_address }` in, `AuthResult` out, scoping namespaces and allowed/forbidden function ids). The engine-owned listener must be a **public, RBAC-protected
  instance** (e.g. `iii-worker-manager#browser`); the private listener must never face
  untrusted browser clients.
- **Never pin `workerName` across tabs.** The default `browser:<random>` is deliberate: the
  engine allows one live worker per name per namespace, so two tabs sharing a fixed name evict
  each other. Pin it only for a deliberately single-tab app that needs a stable identity.
- **Surface connection state.** `addConnectionStateListener(handler)` fires immediately with
  the current state and then on every transition (`disconnected`, `connecting`, `connected`,
  `reconnecting`, `failed`); it returns an unsubscribe function. Bind the reconnecting banner
  and stale-data treatment to it. Reconnection is exponential backoff with jitter — tune
  `reconnectionConfig` (`initialDelayMs` 1000, `maxDelayMs` 30000, `backoffMultiplier` 2,
  `jitterFactor` 0.3, `maxRetries` -1) rather than writing your own retry loop.
- **Fatal rejections are terminal.** A name or namespace collision raises
  `RegistrationRejectedError` (`code`, `namespace`, `worker_name`, `function_id`,
  `owner_worker_id`), is thrown into every pending invocation, and closes the connection for
  good; `iii.getFatalError()` returns it while healthy returns `undefined`. Show it; do not
  retry blindly.
- **`shutdown()`** on teardown, in tests, and before re-registering during a hot reload.

## React integration

One worker per app, created outside React and handed down. Never one per component, and never
inside a render body — StrictMode double-invokes render, so a socket built there leaks.

```ts
// src/iii/client.ts
import { registerWorker, type ISdk } from 'iii-browser-sdk'

let client: ISdk | undefined
export function getIii(): ISdk {
  return (client ??= registerWorker(import.meta.env.VITE_III_WS_URL))
}
```

```tsx
// src/iii/IiiProvider.tsx
export const useIii = () => {
  const iii = useContext(IiiContext)
  if (!iii) throw new Error('useIii must be used inside <IiiProvider>')
  return iii
}

export function IiiProvider({ children }: { children: ReactNode }) {
  const iii = getIii()
  const [state, setState] = useState<IIIConnectionState>('connecting')
  useEffect(() => iii.addConnectionStateListener(setState), [iii])
  const fatal = iii.getFatalError()
  if (fatal) return <FatalBanner error={fatal} />
  return (
    <IiiContext.Provider value={iii}>
      <ReconnectBanner state={state} />
      {children}
    </IiiContext.Provider>
  )
}
```

Rules that keep this honest:

- **Register handlers and observers in an effect; remove them in its cleanup.**
  `registerFunction` returns a `FunctionRef` with `unregister()`, and `observe` an
  `ObserverHandle` with `dispose()`. Skip them and an unmount or HMR leaves a stale observer
  delivering to a dead handler.
- **Function ids must be stable and addressable.** Name them for what they do
  (`ui::notification`), never after a component instance. One connection hosts an id per
  namespace, so a function every tab registers carries a per-tab suffix; use a `state` key or
  a channel when a backend needs one specific tab.
- **`call()` is an imperative async call, not a render input.** Route it through a query or
  mutation layer (`queryFn: () => iii.call({ ... })`) so caching, retries, and loading
  states live in one place. Never call it during render.
- **Server state is not UI state.** Engine state and call results belong in the cache, not
  mirrored into `useState`; an observer's handler updates or invalidates that cache.
- **Errors are values.** A rejected `call` should reach an error boundary or an inline
  state, not be swallowed in a `.catch(() => {})`.

## Vite

```bash
# .env.local
VITE_III_WS_URL=ws://localhost:3111
```

- Only `VITE_`-prefixed variables are exposed; read them with `import.meta.env.*`.
- The package is ESM with `exports` for `.`, `./helpers`, `./state`, and `./stream`;
  `import` and `require` both resolve, and no polyfill is needed on native `WebSocket`.
- If a dev-server reload ever races the first registration (symptoms: two clients, two
  sockets, a duplicated module), add `optimizeDeps: { include: ['iii-browser-sdk'] }` and clear
  `node_modules/.vite` once.
- A production build pointing at `ws://localhost` is a silent failure that only shows up in the
  connection banner. Keep the URL in the environment, never committed to source.

## Checklist before claiming it works

1. `engine::functions::list { prefix: "ui::" }` lists the functions the tab registered.
2. `engine::functions::info { function_id: "ui::…" }` returns the description you passed.
3. A real backend call of that id, or of the function the tab observes, changes the rendered
   UI — seen in a live `browser::sessions::start` tab, not assumed from the code.
4. Stopping the engine flips the UI to `reconnecting`; restarting it restores registration with
   no page reload.
5. No stale handler or observer survives an unmount or HMR (`unregister()` and `dispose()`
   run in cleanup); `engine::observers::list` shows none left behind.
6. A name or namespace collision is surfaced through `getFatalError()`, not swallowed.
