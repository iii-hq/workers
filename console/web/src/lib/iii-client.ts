/**
 * Lazy-initialized singleton iii-browser-sdk client used by console/web.
 *
 * Bootstrap order:
 *   1. Resolve the engine WebSocket URL from `VITE_ENGINE_WS_URL` or a
 *      relative-path default against `window.location.host`. There is no
 *      HTTP bootstrap — the browser opens a single WebSocket directly to
 *      the engine (dev: through the Vite `/ws` proxy in
 *      `console/web/vite.config.ts`; prod: through the `console` worker
 *      which proxies `/ws` to the engine).
 *   2. Open the WebSocket via `iii-browser-sdk::registerWorker(url)`.
 *   3. Mint a stable `browser_id` for this page; per-browser handlers are
 *      registered under `<functionId>::<browserId>` so engine triggers — the
 *      browser's own scoped stream subscriptions and the harness sessions
 *      fan-out — deliver to this specific browser's connection.
 *
 * Once `_clientPromise` is resolved, every other call (`trigger`, `on`,
 * `dispose`) goes over the single WS connection.
 */

import {
  type IIIConnectionState,
  type ISdk,
  type RegisterFunctionOptions,
  type RegisterTriggerInput,
  type RemoteFunctionHandler,
  registerWorker,
} from 'iii-browser-sdk'

export type { IIIConnectionState, RegisterTriggerInput }

export interface IiiClient {
  browserId: string
  /**
   * Invoke an iii bus function and await its result. In the iii ecosystem
   * every bus invocation is a *trigger* — there is no separate "call"
   * concept. Thin wrapper over the SDK's `trigger({ function_id, payload })`.
   */
  trigger<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
  ): Promise<T>
  on<P = unknown>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
    options?: RegisterFunctionOptions,
  ): () => void
  /**
   * Register an engine trigger bound to a function id. Thin passthrough to
   * the SDK's `registerTrigger`; returns an unregister fn that is also run on
   * `dispose()`. Pair with `on()` + `browserId` to bind a trigger to a
   * browser-local handler:
   *   const off = client.on('foo', handler)
   *   const offTrigger = client.registerTrigger({
   *     type: 'worker',
   *     function_id: `foo::${client.browserId}`,
   *     config: { operations: ['add'] },
   *   })
   */
  registerTrigger(input: RegisterTriggerInput): () => void
  addConnectionStateListener(
    handler: (state: IIIConnectionState) => void,
  ): () => void
  dispose(): Promise<void>
}

interface Deps {
  resolveWsUrl: () => string
  makeBrowserId: () => string
  registerWorker: (url: string) => ISdk
}

let _clientPromise: Promise<IiiClient> | null = null
let _deps: Deps = defaultDeps()

function defaultDeps(): Deps {
  return {
    resolveWsUrl,
    makeBrowserId,
    registerWorker: (url) => registerWorker(url),
  }
}

/**
 * Get (and lazily create) the shared iii-client for this page.
 */
export function getIiiClient(): Promise<IiiClient> {
  if (!_clientPromise) {
    _clientPromise = bootstrap(_deps)
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
  const browserId = deps.makeBrowserId()
  const sdk = deps.registerWorker(wsUrl)
  return wrapSdk(sdk, browserId)
}

function wrapSdk(sdk: ISdk, browserId: string): IiiClient {
  // Track per-functionId unregister fns so dispose() can clean them all up
  // even if individual `on()` callers forgot.
  const handlerUnregisters = new Set<() => void>()
  // Same idea for trigger registrations, so an unmount that forgets to
  // unregister still releases the engine-side binding on dispose().
  const triggerUnregisters = new Set<() => void>()

  function trigger<T>(
    functionId: string,
    payload: Record<string, unknown> = {},
  ): Promise<T> {
    return sdk.trigger<unknown, T>({
      function_id: functionId,
      payload,
    })
  }

  function on<P>(
    functionId: string,
    handler: (payload: P) => void | Promise<void>,
    options?: RegisterFunctionOptions,
  ): () => void {
    const id = `${functionId}::${browserId}`
    // Wrap to satisfy the SDK's RemoteFunctionHandler signature (returns
    // Promise<unknown>). We never want to send a result back.
    const wrapped: RemoteFunctionHandler = async (data: unknown) => {
      await handler(data as P)
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
    let active = true
    const unregister = () => {
      if (!active) return
      active = false
      handlerUnregisters.delete(unregister)
      try {
        ref.unregister()
      } catch {
        // SDK already disposed; nothing to do.
      }
    }
    handlerUnregisters.add(unregister)
    return unregister
  }

  function registerTrigger(input: RegisterTriggerInput): () => void {
    const trigger = sdk.registerTrigger(input)
    let active = true
    const unregister = () => {
      if (!active) return
      active = false
      triggerUnregisters.delete(unregister)
      try {
        trigger.unregister()
      } catch {
        // SDK already disposed; nothing to do.
      }
    }
    triggerUnregisters.add(unregister)
    return unregister
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
    for (const unregister of [...triggerUnregisters]) {
      unregister()
    }
    await sdk.shutdown()
  }

  return {
    browserId,
    trigger,
    on,
    registerTrigger,
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

function makeBrowserId(): string {
  // crypto.randomUUID() is available in all evergreen browsers.
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `console-${crypto.randomUUID()}`
  }
  // Fallback for environments without crypto.randomUUID (older WebViews).
  const rand = Math.random().toString(36).slice(2, 10)
  return `console-${Date.now().toString(36)}-${rand}`
}
