/**
 * Live subscriptions to session-manager's six trigger types, following the
 * integration contract (register a browser-local handler, then bind a
 * trigger of that type to it). Delivery is at-least-once and unordered —
 * consumers reconcile by `revision` per entry and treat reads as the source
 * of truth.
 *
 * Two granularities:
 * - `subscribeSessionDirectory`: sidebar-level events (created, meta-updated,
 *   status-changed, deleted) across ALL sessions.
 * - `subscribeSessionTranscript`: message-added / message-updated for ONE
 *   session (`config.session_id` filter applied engine-side).
 *
 * Handler ids carry the `iii::` prefix so delivery spans are tagged internal
 * and hidden from the Traces view (same convention as session-events-live).
 */

import type { IiiClient } from '@/lib/iii-client'
import type {
  MessageAddedEvent,
  MessageUpdatedEvent,
  MetaUpdatedEvent,
  SessionCreatedEvent,
  SessionDeletedEvent,
  SessionTriggerType,
  StatusChangedEvent,
} from './types'

const HANDLER_BASE = 'iii::console::session'

type ClientSubset = Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>

function bind<P>(
  client: ClientSubset,
  triggerType: SessionTriggerType,
  config: Record<string, unknown>,
  onEvent: (payload: P) => void,
  /** Distinguishes per-session bindings from the global ones. */
  scope: string,
): () => void {
  const localFnId = `${HANDLER_BASE}_${triggerType.replace(/::/g, '_')}_${scope}`
  const offHandler = client.on(localFnId, (payload: unknown) => {
    if (payload && typeof payload === 'object') onEvent(payload as P)
  })
  // `on()` registers under `<fn>::<browserId>`; the trigger must target that id.
  const offTrigger = client.registerTrigger({
    type: triggerType,
    function_id: `${localFnId}::${client.browserId}`,
    config,
  })
  return () => {
    offHandler()
    try {
      offTrigger()
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
    offs.push(bind(client, 'session::created', {}, handlers.onCreated, 'dir'))
  }
  if (handlers.onMetaUpdated) {
    offs.push(
      bind(client, 'session::meta-updated', {}, handlers.onMetaUpdated, 'dir'),
    )
  }
  if (handlers.onStatusChanged) {
    offs.push(
      bind(
        client,
        'session::status-changed',
        {},
        handlers.onStatusChanged,
        'dir',
      ),
    )
  }
  if (handlers.onDeleted) {
    offs.push(bind(client, 'session::deleted', {}, handlers.onDeleted, 'dir'))
  }
  return () => {
    for (const off of offs) off()
  }
}

export interface SessionTranscriptHandlers {
  onMessageAdded: (event: MessageAddedEvent) => void
  onMessageUpdated: (event: MessageUpdatedEvent) => void
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
  const config = { session_id: sessionId }
  const guard =
    <P extends { session_id: string }>(fn: (event: P) => void) =>
    (event: P) => {
      // Defense-in-depth: the trigger filter is already session-scoped.
      if (event.session_id === sessionId) fn(event)
    }
  const offs = [
    bind(
      client,
      'session::message-added',
      config,
      guard(handlers.onMessageAdded),
      'live',
    ),
    bind(
      client,
      'session::message-updated',
      config,
      guard(handlers.onMessageUpdated),
      'live',
    ),
  ]
  return () => {
    for (const off of offs) off()
  }
}
