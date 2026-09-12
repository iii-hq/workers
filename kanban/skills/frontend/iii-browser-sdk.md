---
title: iii-browser-sdk
description: Make a browser app a first-class iii worker with iii-browser-sdk — registerWorker, registerFunction, trigger, registerTrigger, engine state, and streaming channels, wired into React and Vite.
type: how-to
---

# iii-browser-sdk

`iii-browser-sdk` turns the frontend into an iii worker: one WebSocket to the engine instead
of many HTTP round-trips, backend workers pushing into the UI with `trigger()`, and the same
primitives used server-side (`registerFunction`, `trigger`, `registerTrigger`). No Node.js
dependencies, no OpenTelemetry — it runs on native `WebSocket`.

Docs <https://iii.dev/docs/how-to/use-iii-in-the-browser> ·
API reference <https://iii.dev/docs/api-reference/sdk-browser> ·
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
the returned `ISdk` — `registerFunction`, `registerTrigger`, `registerTriggerType`,
`trigger`, `addConnectionStateListener`, `getFatalError`, `shutdown`. Do not destructure
these as top-level imports; they are not exported that way.

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
- A backend worker reaches it by id: `iii.trigger({ function_id: 'ui::show-notification', payload })`.

## Calling backend functions

```ts
const users = await iii.trigger<{ limit: number }, User[]>({
  function_id: 'api::get::users',
  payload: { limit: 20 },
  timeoutMs: 5000,
})
```

Three routing modes — choose deliberately:

| Mode | Call | Use when |
| --- | --- | --- |
| await (default) | `await iii.trigger({ function_id, payload })` | you need the result |
| fire-and-forget | `action: TriggerAction.Void()` | telemetry, non-essential side effects |
| enqueue | `action: TriggerAction.Enqueue({ queue })` | long work; resolves to `{ messageReceiptId }` |

`namespace` on the request routes the invocation elsewhere; omit it to stay in your own.

## Receiving live pushes (there is no polling story)

The engine can invoke a function registered in the tab at any time:

```ts
iii.registerFunction('ui::update-dashboard', async (m: Metrics) => {
  store.setMetrics(m)
  return null
})
```

That is the whole real-time design: a backend worker calls
`trigger({ function_id: 'ui::update-dashboard', payload })` and the open tab updates. Do not
add polling, SSE, or a second socket alongside it.

## Triggers in the browser

```ts
const trigger = iii.registerTrigger({
  type: 'state',
  function_id: 'ui::refresh', // a function this tab registered
  config: { scope: 'orders', key: '*' },
})
trigger.unregister()
```

`registerTrigger` fills `namespace` from this worker — correct, because the target function is
one this worker registered. Registering a **trigger type** (`registerTriggerType`) from a
browser is rarely right: it only makes sense when the browser itself is the event source. The
RBAC listener also constrains which types a browser session may register.

## State, helpers, channels (subpath exports)

- `iii-browser-sdk/state` — the engine's state worker, typed: `get<T>`/`set`/`delete`/`list`/
  `update`, plus `StateEventType.Created|Updated|Deleted` for the change events the `state`
  trigger delivers. Use `update` with ordered `ops` when you need an atomic edit instead of a
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
  ip_address }` in, `AuthResult` out, scoping namespaces, allowed/forbidden function ids, and
  allowed trigger types). The engine-owned listener must be a **public, RBAC-protected
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

- **Register handlers in an effect; unregister in its cleanup.** `registerFunction` returns a
  `FunctionRef` with `unregister()`. Skip it and an unmount or HMR leaves a stale handler
  answering backend calls in a dead tab.
- **Function ids must be stable and addressable.** Name them for what they do
  (`ui::notification`), never after a component instance. Two tabs registering the same id
  load-balance, so target a *function*; use a `state` key or a channel when you need one
  specific tab.
- **`trigger()` is an imperative async call, not a render input.** Route it through a query or
  mutation layer (`queryFn: () => iii.trigger({ ... })`) so caching, retries, and loading
  states live in one place. Never call it during render.
- **Server state is not UI state.** Engine state and trigger results belong in the cache, not
  mirrored into `useState`.
- **Errors are values.** A rejected `trigger` should reach an error boundary or an inline
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
3. A real backend `trigger` of that id changes the rendered UI — observed in a live
   `browser::sessions::start` tab, not assumed from the code.
4. Stopping the engine flips the UI to `reconnecting`; restarting it restores registration with
   no page reload.
5. No stale handler survives an unmount or HMR (the `registerFunction` cleanup is present).
6. A name or namespace collision is surfaced through `getFatalError()`, not swallowed.
