import { useCallback } from 'react'
import { openMentionSession } from '@/lib/mentions/open-session'
import type { MentionOpen } from '@/lib/mentions/types'
import { requestPanelOpen, requestScreenOpen } from '@/lib/panel-context'
import { requestTraceFocus } from '@/lib/trace-focus'
import { isValidScreen } from '@/lib/workspace-tabs'

function contextString(context: unknown, key: string): string | null {
  if (!context || typeof context !== 'object' || Array.isArray(context))
    return null
  const value = (context as Record<string, unknown>)[key]
  return typeof value === 'string' && value ? value : null
}

/**
 * Open what a mention points at:
 * - `{ url }`: a new browser tab (http/https only);
 * - `{ session }`: that chat;
 * - `{ page: 'traces', context: { trace_id } }`: the traces screen with
 *   that trace expanded (any other built-in screen just opens);
 * - `{ page, context }`: a worker page, handed its context.
 *
 * Returns whether anything opened.
 */
export function openMentionTarget(open: MentionOpen): boolean {
  if ('url' in open) {
    if (!/^https?:\/\//i.test(open.url)) return false
    window.open(open.url, '_blank', 'noopener,noreferrer')
    return true
  }
  if ('session' in open) return openMentionSession(open.session)
  if (isValidScreen(open.page) && !open.page.startsWith('ext:')) {
    requestScreenOpen({ screen: open.page })
    const traceId =
      open.page === 'traces' ? contextString(open.context, 'trace_id') : null
    if (traceId) requestTraceFocus(traceId)
    return true
  }
  requestPanelOpen({ pageId: open.page, context: open.context ?? null })
  return true
}

/** `openMentionTarget` for an optional target. */
export function useOpenMention(): (open: MentionOpen | undefined) => boolean {
  return useCallback(
    (open: MentionOpen | undefined) => (open ? openMentionTarget(open) : false),
    [],
  )
}
