/**
 * Live observers of session-manager's six event functions: register a
 * browser-local handler, then observe the function with that handler as the
 * target. Delivery is at-least-once and unordered — consumers reconcile by
 * `revision` per entry and treat reads as the source of truth.
 *
 * Two granularities:
 * - `subscribeSessionDirectory`: sidebar-level events (created, meta-updated,
 *   status-changed, deleted) across ALL sessions.
 * - `subscribeSessionTranscript`: message-added / message-updated for ONE
 *   session (an `input.session_id` filter iii applies before delivery).
 *
 * Handler ids carry the `iii::` prefix so delivery spans are tagged internal
 * and hidden from the Traces view (same convention as turn-events-live).
 */

import type {
  IiiClient,
  ObserveReason,
  ObserverDisposer,
} from '@/lib/iii-client'
import type {
  MessageAddedEvent,
  MessageUpdatedEvent,
  MetaUpdatedEvent,
  SessionCreatedEvent,
  SessionDeletedEvent,
  SessionFunction,
  StatusChangedEvent,
} from './types'

const HANDLER_BASE = 'iii::console::session'
let transcriptSubscriptionSequence = 0

type ClientSubset = Pick<IiiClient, 'browserId' | 'on' | 'observe'>

function bind<P>(
  client: ClientSubset,
  functionId: SessionFunction,
  filters: Record<string, unknown> | undefined,
  onEvent: (payload: P) => void,
  /** Distinguishes per-session observers from the global ones. */
  scope: string,
  /** Called when the engine refuses the observer (its state turns `error`). */
  onRefused?: (reason?: ObserveReason) => void,
): () => void {
  const localFnId = `${HANDLER_BASE}_${functionId.replace(/::/g, '_')}_${scope}`
  const offHandler = client.on(localFnId, (payload: unknown) => {
    if (payload && typeof payload === 'object') onEvent(payload as P)
  })
  // `on()` registers under `<fn>::<browserId>`; the observer targets that id.
  let offObserver: ObserverDisposer
  try {
    offObserver = client.observe(
      { function_id: functionId, ...(filters ? { filters } : {}) },
      `${localFnId}::${client.browserId}`,
    )
  } catch (error) {
    offHandler()
    throw error
  }
  if (onRefused && typeof offObserver.onStateChange === 'function') {
    offObserver.onStateChange((state, reason) => {
      if (state === 'error') onRefused(reason)
    })
  }
  return () => {
    offHandler()
    try {
      offObserver()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

export interface SessionDirectoryHandlers {
  onCreated?: (event: SessionCreatedEvent) => void
  onMetaUpdated?: (event: MetaUpdatedEvent) => void
  onStatusChanged?: (event: StatusChangedEvent) => void
  onDeleted?: (event: SessionDeletedEvent) => void
}

/** Sidebar-level subscription across all sessions. Returns a cleanup fn. */
export function subscribeSessionDirectory(
  client: ClientSubset,
  handlers: SessionDirectoryHandlers,
): () => void {
  const offs: Array<() => void> = []
  if (handlers.onCreated) {
    offs.push(
      bind(client, 'session::on-created', undefined, handlers.onCreated, 'dir'),
    )
  }
  if (handlers.onMetaUpdated) {
    offs.push(
      bind(
        client,
        'session::on-meta-updated',
        undefined,
        handlers.onMetaUpdated,
        'dir',
      ),
    )
  }
  if (handlers.onStatusChanged) {
    offs.push(
      bind(
        client,
        'session::on-status-changed',
        undefined,
        handlers.onStatusChanged,
        'dir',
      ),
    )
  }
  if (handlers.onDeleted) {
    offs.push(
      bind(client, 'session::on-deleted', undefined, handlers.onDeleted, 'dir'),
    )
  }
  return () => {
    for (const off of offs) off()
  }
}

export interface SessionTranscriptHandlers {
  onMessageAdded: (event: MessageAddedEvent) => void
  onMessageUpdated: (event: MessageUpdatedEvent) => void
  /**
   * Called once when the engine refuses one of the observers
   * (`TOO_MANY_LISTENERS`, `FORBIDDEN`, ...): no live messages arrive, so
   * the caller falls back as it does when subscribing throws.
   */
  onObserverError?: (reason?: ObserveReason) => void
}

/**
 * Transcript subscription for one session. Events carry full message
 * snapshots; reconcile by keeping the highest `revision` per entry.
 */
export function subscribeSessionTranscript(
  client: ClientSubset,
  sessionId: string,
  handlers: SessionTranscriptHandlers,
): () => void {
  const filters = { 'input.session_id': { $eq: sessionId } }
  // Several chat panels can observe different sessions at once. Handler ids
  // must therefore be unique per observer; reusing one `live` scope made
  // the second subscription replace/collide with the first in the browser
  // SDK even though their filters differed.
  const scope = `live_${++transcriptSubscriptionSequence}`
  const guard =
    <P extends { session_id: string }>(fn: (event: P) => void) =>
    (event: P) => {
      // Defense-in-depth: the observer's filter is already session-scoped.
      if (event.session_id === sessionId) fn(event)
    }
  const offs: Array<() => void> = []
  // Both observers share one report: the first refusal ends the subscription.
  let refused = false
  const onRefused = handlers.onObserverError
    ? (reason?: ObserveReason) => {
        if (refused) return
        refused = true
        handlers.onObserverError?.(reason)
      }
    : undefined
  try {
    offs.push(
      bind(
        client,
        'session::on-message-added',
        filters,
        guard(handlers.onMessageAdded),
        scope,
        onRefused,
      ),
    )
    offs.push(
      bind(
        client,
        'session::on-message-updated',
        filters,
        guard(handlers.onMessageUpdated),
        scope,
        onRefused,
      ),
    )
  } catch (error) {
    // A failed second observer must not leak the first into every retry.
    for (const off of offs) off()
    throw error
  }
  return () => {
    for (const off of offs) off()
  }
}
