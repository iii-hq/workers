/**
 * Bind an engine trigger type to a browser-local handler: the generic form of
 * the `client.on()` + `client.registerTrigger()` pair, for any trigger type a
 * worker (or the engine) publishes — `compose-operation`,
 * `engine::workers-available`, `browser::chromium-install-progress`, ...
 *
 * Every subscription registers its own handler id, so two waits running at
 * once (two setup steps, two tabs) never share or steal each other's events.
 * The id lives under `iii::console::` so the deliveries stay out of the
 * Traces view, like the console's other plumbing handlers.
 *
 * The engine accepts the binding asynchronously: an event the provider emits
 * in the instant between this call and the binding landing can be missed.
 * Callers subscribe BEFORE they start the work they want to follow, and
 * read the state once afterwards to cover that window — never a loop.
 */

import { getIiiClient, type IiiClient } from '@/lib/iii-client'

type TriggerClient = Pick<IiiClient, 'on' | 'registerTrigger' | 'browserId'>

let sequence = 0

/** A per-subscription handler id under `base`, unique within this page. */
export function uniqueHandlerId(base: string): string {
  sequence += 1
  const nonce = Math.random().toString(36).slice(2, 8)
  return `${base}::${sequence}${nonce}`
}

export interface SubscribeOptions {
  /** Base for the browser-local handler id (`iii::console::<what>`). */
  handler?: string
  /** Test seam; defaults to the shared page client. */
  client?: TriggerClient
}

/**
 * Subscribe `onEvent` to `triggerType` with `config`. Resolves to a disposer
 * that drops both the engine binding and the local handler (idempotent).
 * Rejects when the client cannot register — the caller decides whether a
 * missing subscription is fatal or just means it relies on its fallback read.
 */
export async function subscribeEngineTrigger<P = unknown>(
  triggerType: string,
  config: Record<string, unknown>,
  onEvent: (payload: P) => void,
  options: SubscribeOptions = {},
): Promise<() => void> {
  const client = options.client ?? (await getIiiClient())
  const fnId = uniqueHandlerId(options.handler ?? 'iii::console::trigger')
  let offHandler: (() => void) | undefined
  let offTrigger: (() => void) | undefined
  try {
    offHandler = client.on<P>(fnId, (payload) => {
      onEvent(payload)
    })
    offTrigger = client.registerTrigger({
      type: triggerType,
      function_id: `${fnId}::${client.browserId}`,
      config,
    })
  } catch (error) {
    offHandler?.()
    throw error
  }
  let disposed = false
  return () => {
    if (disposed) return
    disposed = true
    try {
      offTrigger?.()
    } catch {
      // SDK already disposed; nothing to do.
    }
    offHandler?.()
  }
}
