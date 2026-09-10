/**
 * Ephemeral context bridge between worker-owned chat renderers and
 * worker-owned workspace pages.
 *
 * `requestPanelOpen` stores the latest event before notifying the workspace,
 * so a page mounted by that request receives its context on first render. The
 * workspace layout stays host-owned; extensions only name a registered page
 * and provide JSON context.
 */

import { useCallback, useSyncExternalStore } from 'react'
import type {
  JsonValue,
  PanelContextEvent,
  PanelOpenRequest,
} from '@/types/injectable-ui'

type OpenListener = (event: PanelContextEvent) => void

let nextEventId = 1
let contexts = new Map<string, PanelContextEvent>()
const contextListeners = new Set<() => void>()
const openListeners = new Set<OpenListener>()

function emitContexts(): void {
  for (const listener of [...contextListeners]) listener()
}

function subscribeContexts(listener: () => void): () => void {
  contextListeners.add(listener)
  return () => contextListeners.delete(listener)
}

/** Called by an injectable Host. Context is intentionally JSON-only. */
export function requestPanelOpen(request: PanelOpenRequest): PanelContextEvent {
  const event: PanelContextEvent = {
    id: nextEventId++,
    pageId: request.pageId,
    context: request.context ?? null,
  }
  contexts = new Map(contexts).set(request.pageId, event)
  emitContexts()
  for (const listener of [...openListeners]) listener(event)
  return event
}

/** The workspace subscribes here to place or reuse the requested page. */
export function subscribePanelOpen(listener: OpenListener): () => void {
  openListeners.add(listener)
  return () => openListeners.delete(listener)
}

export function getPanelContext(
  pageId: string,
): PanelContextEvent<JsonValue> | undefined {
  return contexts.get(pageId)
}

/** Reactive latest context for a mounted injected page. */
export function usePanelContext(
  pageId: string,
): PanelContextEvent<JsonValue> | undefined {
  const getSnapshot = useCallback(() => getPanelContext(pageId), [pageId])
  return useSyncExternalStore(subscribeContexts, getSnapshot, () => undefined)
}

/** Test-only reset; exported to keep the store deterministic in unit tests. */
export function resetPanelContextForTests(): void {
  nextEventId = 1
  contexts = new Map()
  contextListeners.clear()
  openListeners.clear()
}
