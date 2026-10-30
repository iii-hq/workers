/**
 * Lazy-initialized singleton iii-browser-sdk client used by ade/web.
 *
 * Bootstrap order:
 *   1. Resolve the engine WebSocket URL from `VITE_ENGINE_WS_URL` or a
 *      relative-path default against `window.location.host`. There is no
 *      HTTP bootstrap — the browser opens a single WebSocket directly to
 *      the engine (dev: through the Vite `/ws` proxy in
 *      `ade/web/vite.config.ts`; prod: through the `console` worker
 *      which proxies `/ws` to the engine).
 *   2. Open the WebSocket via `iii-browser-sdk::registerWorker(url)`.
 *   3. Mint a stable `browser_id` for this page; per-browser handlers are
 *      registered under `<functionId>::<browserId>`, and the tab's
 *      observers name them as their target, so each call a tab observes is
 *      delivered to this specific browser's connection.
 *
 * Once `_clientPromise` is resolved, every other call (`call`, `on`,
 * `observe`, `dispose`) goes over the single WS connection.
 */

import {
  type IIIConnectionState,
  type InitOptions,
  type ISdk,
  type RegisterFunctionOptions,
  type RemoteFunctionHandler,
  registerWorker,
  type ObserveSource as SdkObserveSource,
} from 'iii-browser-sdk'

export type { IIIConnectionState }

/** The function a tab observes, with its args and filters. */
export interface ObserveSource {
  function_id: string
  /**
   * Unset, the namespace resolves as for a call; the engine's own sources
   * (`engine::*`, `configuration::*`, `observability::*`, `stream::*`) are
   * observed in `default`.
   */
  namespace?: string
  /** For a function that takes args; its worker's README lists them. */
  args?: Record<string, unknown>
  /**
   * Select calls by `input`, `output`, `error`, `caller` and `metadata`,
   * each with an explicit operator: `{ 'input.session_id': { $eq: id } }`.
   */
  filters?: Record<string, unknown>
  /** What the target receives: the event (`'output'`, the default), the call's `'input'`, or the `'envelope'`. */
  deliver?: 'envelope' | 'input' | 'output'
}

/** Observer options: an explicit id, the metadata sidecar the target receives, and the policy. */
export interface ObserverOptions {
  id?: string
  metadata?: Record<string, unknown>
  max_fires?: number
  expires_in_ms?: number
  max_in_flight?: number
}

export interface CallOptions {
  timeoutMs?: number
  namespace?: string
}

export interface IiiClient {
  browserId: string
  /**
   * Call an iii function and await its result. Thin wrapper over the SDK's
   * `call({ function_id, payload })`.
   *
   * The client's default timeout is 5 minutes; pass `timeoutMs` for
   * functions that legitimately run longer (e.g. `context::compact`, whose
   * summariser call is budgeted up to 320s).
   */
  call<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
    options?: CallOptions,
  ): Promise<T>
  /** @deprecated the same function as `call`, for injected UI built against older consoles */
  trigger: IiiClient['call']
  on<P = unknown>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
    options?: RegisterFunctionOptions,
  ): () => void
  /**
   * Observe `source` with `target`, a function registered with `on()` by
   * its full id. Returns its disposer, which `dispose()` also runs.
   * `deliver` defaults to `'output'`, so the handler receives the event the
   * observed function returned:
   *   const off = client.on('foo', handler)
   *   const offObserver = client.observe(
   *     { function_id: 'session::on-created', filters: { 'input.session_id': { $eq: id } } },
   *     `foo::${client.browserId}`,
   *   )
   * The observer stays pending until its function registers, so observing a
   * source whose worker is absent never fails here.
   */
  observe(
    source: string | ObserveSource,
    target: string,
    options?: ObserverOptions,
  ): () => void
  addConnectionStateListener(
    handler: (state: IIIConnectionState) => void,
  ): () => void
  dispose(): Promise<void>
}

interface Deps {
  resolveWsUrl: () => string
  resolveNamespace: () => Promise<string | undefined>
  makeBrowserId: () => string
  registerWorker: (url: string, options?: InitOptions) => ISdk
}

let _clientPromise: Promise<IiiClient> | null = null
let _deps: Deps = defaultDeps()

function defaultDeps(): Deps {
  return {
    resolveWsUrl,
    resolveNamespace,
    makeBrowserId,
    registerWorker: (url, options) => registerWorker(url, options),
  }
}

/**
 * Get (and lazily create) the shared iii-client for this page.
 */
export function getIiiClient(): Promise<IiiClient> {
  if (!_clientPromise) {
    _clientPromise = bootstrap(_deps).catch((error: unknown) => {
      _clientPromise = null
      throw error
    })
  }
  return _clientPromise
}

/**
 * Tear down the shared client. Safe to call multiple times.
 */
export async function disposeIiiClient(): Promise<void> {
  if (!_clientPromise) return
  const client = await _clientPromise.catch(() => null)
  _clientPromise = null
  if (client) await client.dispose()
}

/**
 * Test seam: replace the bootstrap dependencies and wipe the cached client.
 * Tests should call `__resetIiiClientForTests()` in afterEach to restore.
 */
export function __setIiiClientDepsForTests(overrides: Partial<Deps>): void {
  _deps = { ..._deps, ...overrides }
  _clientPromise = null
}

export function __resetIiiClientForTests(): void {
  _deps = defaultDeps()
  _clientPromise = null
}

async function bootstrap(deps: Deps): Promise<IiiClient> {
  const wsUrl = deps.resolveWsUrl()
  const namespace = await deps.resolveNamespace()
  const browserId = deps.makeBrowserId()
  const sdk = deps.registerWorker(
    wsUrl,
    namespace === undefined ? undefined : { namespace },
  )
  return wrapSdk(sdk, browserId)
}

export function wrapSdk(sdk: ISdk, browserId: string): IiiClient {
  // Track per-functionId unregister fns so dispose() can clean them all up
  // even if individual `on()` callers forgot.
  const handlerUnregisters = new Set<() => void>()
  // Same idea for observers, so an unmount that forgets to dispose still
  // releases the engine-side observer on dispose().
  const observerDisposers = new Set<() => void>()

  const DEFAULT_CALL_TIMEOUT_MS = 5 * 60 * 1000

  function call<T>(
    functionId: string,
    payload: Record<string, unknown> = {},
    options?: CallOptions,
  ): Promise<T> {
    const namespace = options?.namespace ?? controlPlaneNamespace(functionId)
    return sdk.call<unknown, T>({
      function_id: functionId,
      payload,
      timeoutMs: options?.timeoutMs ?? DEFAULT_CALL_TIMEOUT_MS,
      ...(namespace === undefined ? {} : { namespace }),
    })
  }

  // One SDK registration per function id, fanned out to every attached
  // listener. The SDK throws on a duplicate id, but two live streams
  // legitimately bind the same constant handler id (e.g. a still-streaming
  // session's turn-completed handler alongside a new session's) — letting
  // that throw killed the new session's send silently. The SDK registration
  // is released when the last listener detaches.
  const fanouts = new Map<
    string,
    {
      listeners: Set<(data: unknown) => void | Promise<void>>
      release: () => void
    }
  >()

  function on<P>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
    options?: RegisterFunctionOptions,
  ): () => void {
    const id = `${functionId}::${browserId}`
    let entry = fanouts.get(id)
    if (!entry) {
      const listeners = new Set<(data: unknown) => void | Promise<void>>()
      // Wrap to satisfy the SDK's RemoteFunctionHandler signature (returns
      // Promise<unknown>). We never want to send a result back, and one
      // listener's failure must not starve the others.
      const wrapped: RemoteFunctionHandler = async (data: unknown) => {
        await Promise.allSettled([...listeners].map(async (fn) => fn(data)))
        return null
      }
      // Every handler registered here is a browser-local console plumbing fn
      // (traces, sessions, worktree events, ...) — none are meant for other
      // workers (e.g. harness) to discover, so default them out of
      // `engine::functions::list`. Callers can still override via `options`.
      const ref = sdk.registerFunction(id, wrapped, {
        metadata: { internal: true },
        ...options,
      })
      entry = {
        listeners,
        release: () => {
          try {
            ref.unregister()
          } catch {
            // SDK already disposed; nothing to do.
          }
        },
      }
      fanouts.set(id, entry)
    }
    const fanout = entry
    const listener = (data: unknown) => handler(data as P)
    fanout.listeners.add(listener)
    let active = true
    const unregister = () => {
      if (!active) return
      active = false
      handlerUnregisters.delete(unregister)
      fanout.listeners.delete(listener)
      if (fanout.listeners.size === 0) {
        fanouts.delete(id)
        fanout.release()
      }
    }
    handlerUnregisters.add(unregister)
    return unregister
  }

  function observe(
    source: string | ObserveSource,
    target: string,
    options?: ObserverOptions,
  ): () => void {
    const spec: ObserveSource =
      typeof source === 'string' ? { function_id: source } : source
    const namespace = spec.namespace ?? sourceNamespace(spec.function_id)
    const handle = sdk.observe(
      {
        ...spec,
        deliver: spec.deliver ?? 'output',
        ...(namespace === undefined ? {} : { namespace }),
      } as SdkObserveSource,
      target,
      options,
    )
    let active = true
    const disposeObserver = () => {
      if (!active) return
      active = false
      observerDisposers.delete(disposeObserver)
      handle.dispose().catch(() => {
        // SDK already disposed; the engine drops the observer with the tab.
      })
    }
    observerDisposers.add(disposeObserver)
    return disposeObserver
  }

  function addConnectionStateListener(
    handler: (state: IIIConnectionState) => void,
  ): () => void {
    return sdk.addConnectionStateListener(handler)
  }

  async function dispose(): Promise<void> {
    for (const unregister of [...handlerUnregisters]) {
      unregister()
    }
    for (const disposeObserver of [...observerDisposers]) {
      disposeObserver()
    }
    await sdk.shutdown()
  }

  return {
    browserId,
    call,
    trigger: call,
    on,
    observe,
    addConnectionStateListener,
    dispose,
  }
}

/**
 * Resolve the engine WebSocket URL. Browser preference order:
 *   1. `VITE_ENGINE_WS_URL` if set at build time.
 *   2. Relative path against `window.location` (`wss://` for HTTPS pages,
 *      `ws://` otherwise) so dev hits the Vite `/ws` proxy and prod hits
 *      the `console` worker (or whatever reverse proxy) forwarding `/ws`
 *      to the engine.
 * Outside a browser (tests), default to `ws://127.0.0.1:49134`.
 */
function resolveWsUrl(): string {
  const override = import.meta.env?.VITE_ENGINE_WS_URL as string | undefined
  if (typeof override === 'string' && override.length > 0) {
    return override
  }
  if (typeof window !== 'undefined' && window.location) {
    const url = new URL('./ws', window.location.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    return url.href
  }
  return 'ws://127.0.0.1:49134'
}

/**
 * Read the namespace from the console worker that serves this SPA. Browser
 * clients have no `III_NAMESPACE` environment variable, so the namespace must
 * be passed explicitly to iii-browser-sdk 0.23.
 */
async function resolveNamespace(): Promise<string | undefined> {
  if (typeof window === 'undefined' || !window.location) return undefined
  const url = new URL('./runtime', window.location.href)
  let response: Response
  try {
    response = await fetch(url, {
      cache: 'no-store',
      signal: AbortSignal.timeout(10_000),
    })
  } catch (cause) {
    throw new Error(
      'Runtime namespace request failed; refusing to connect to default',
      { cause },
    )
  }
  if (!response.ok) {
    throw new Error(
      `Runtime namespace request failed with HTTP ${response.status}; refusing to connect to default`,
    )
  }
  const value = (await response.json()) as { namespace?: unknown }
  if (value.namespace === null || value.namespace === undefined)
    return undefined
  if (typeof value.namespace === 'string' && value.namespace.length > 0) {
    return value.namespace
  }
  throw new Error(
    'Runtime namespace response is invalid; refusing to connect to default',
  )
}

/** Engine-owned functions (control plane, iii-sandbox) are registered only in `default`. */
function controlPlaneNamespace(functionId: string): string | undefined {
  return /^(configuration|engine|worker|sandbox)::/.test(functionId)
    ? 'default'
    : undefined
}

/** The engine's own sources are hosted only in `default`. */
function sourceNamespace(functionId: string): string | undefined {
  return /^(configuration|engine|observability|stream)::/.test(functionId)
    ? 'default'
    : undefined
}

function makeBrowserId(): string {
  // crypto.randomUUID() only exists in secure contexts (https / localhost);
  // http://<LAN-IP> origins have `crypto` without it, hence the guard.
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `console-${crypto.randomUUID()}`
  }
  const rand = Math.random().toString(36).slice(2, 10)
  return `console-${Date.now().toString(36)}-${rand}`
}
