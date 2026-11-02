/**
 * Observe a function with a browser-local handler: the generic form of the
 * `client.on()` + `client.observe()` pair, for any function a worker (or the
 * engine) hosts — `compose::on-operation-progress`,
 * `engine::on-workers-changed`, `browser::on-chromium-install-progress`, ...
 *
 * Every subscription registers its own handler id, so two waits running at
 * once (two setup steps, two tabs) never share or steal each other's events.
 * The id lives under `iii::console::` so the deliveries stay out of the
 * Traces view, like the console's other plumbing handlers.
 *
 * The observer becomes active asynchronously: a call the function makes in
 * the instant between this call and the observer turning active can be
 * missed, and an observer of a function whose worker is absent stays
 * pending until it registers. Callers subscribe BEFORE they start the work
 * they want to follow, and read the state once afterwards to cover that
 * window — never a loop. The disposer carries the observer's `state`,
 * `ready()` and `onStateChange` (when the client has them), so that read can
 * wait for it to be active through `catchUpWhenActive`.
 */

import {
  getIiiClient,
  type IiiClient,
  type ObserverDisposer,
  type ObserverOptions,
} from '@/lib/iii-client'
import type { CatchUpObserver } from '@/lib/observer-catch-up'

type ObserveClient = Pick<IiiClient, 'on' | 'observe' | 'browserId'>

let sequence = 0

/** A per-subscription handler id under `base`, unique within this page. */
export function uniqueHandlerId(base: string): string {
  sequence += 1
  const nonce = Math.random().toString(36).slice(2, 8)
  return `${base}::${sequence}${nonce}`
}

/** Which calls of the observed function reach the handler. */
export interface ObservedSelection {
  /** For a function that takes args; its worker's README lists them. */
  args?: Record<string, unknown>
  /** Filters on the call, each with an explicit operator. */
  filters?: Record<string, unknown>
  /** The function's namespace when it is not this console's (Compose's). */
  namespace?: string
}

export interface SubscribeOptions {
  /** Base for the browser-local handler id (`iii::console::<what>`). */
  handler?: string
  /** The observer's options (`expires_in_ms`, `metadata`). */
  observer?: ObserverOptions
  /** Test seam; defaults to the shared page client. */
  client?: ObserveClient
}

/**
 * Observe `functionId` with `selection` and run `onEvent` with the event each
 * matching call returned. Resolves to a disposer that disposes the observer
 * and drops the local handler (idempotent). Rejects when the client throws
 * while observing (no connection, an observer id conflict). The engine's
 * refusal (`FORBIDDEN`, `NOT_OBSERVABLE`, `TOO_MANY_LISTENERS`, ...) does
 * not reject: it arrives later as the observer's `error` state, which the
 * client logs, and no event follows — so callers keep their fallback read
 * (a start check, a silence check, a deadline) whatever happens here.
 */
export async function subscribeObserved<P = unknown>(
  functionId: string,
  selection: ObservedSelection,
  onEvent: (payload: P) => void,
  options: SubscribeOptions = {},
): Promise<CatchUpObserver> {
  const client = options.client ?? (await getIiiClient())
  const fnId = uniqueHandlerId(options.handler ?? 'iii::console::observed')
  let offHandler: (() => void) | undefined
  let offObserver: Partial<ObserverDisposer> | undefined
  try {
    offHandler = client.on<P>(fnId, (payload) => {
      onEvent(payload)
    })
    offObserver = client.observe(
      {
        function_id: functionId,
        ...(selection.namespace ? { namespace: selection.namespace } : {}),
        ...(selection.args ? { args: selection.args } : {}),
        ...(selection.filters ? { filters: selection.filters } : {}),
      },
      `${fnId}::${client.browserId}`,
      options.observer,
    )
  } catch (error) {
    offHandler?.()
    throw error
  }
  let disposed = false
  const dispose = () => {
    if (disposed) return
    disposed = true
    try {
      ;(offObserver as (() => void) | undefined)?.()
    } catch {
      // SDK already disposed; nothing to do.
    }
    offHandler?.()
  }
  const observer = offObserver
  if (
    typeof observer?.ready === 'function' &&
    typeof observer.onStateChange === 'function'
  ) {
    const { ready, onStateChange } = observer as ObserverDisposer
    Object.defineProperties(dispose, {
      state: { get: () => observer.state },
      ready: { value: () => ready() },
      onStateChange: {
        value: (listener: Parameters<ObserverDisposer['onStateChange']>[0]) =>
          onStateChange(listener),
      },
    })
  }
  return dispose as CatchUpObserver
}
