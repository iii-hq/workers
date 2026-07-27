import type { Conversation } from '@/types/chat'

/**
 * State surfaced by the chat-dock toggle when the dock is collapsed.
 *
 * - `active`: the assistant is producing output or waiting on the user
 *   (streaming, pending approval, running tool call). Foregrounded with the
 *   accent pulse so blocking events don't get buried.
 * - `attention`: the most recent system message warns the user about
 *   something they should look at (rate limit, soft failure, compaction
 *   notice). Static warn-tinted border, no pulse — calmer than `active`.
 * - `error`: the most recent system message is a hard failure. Static
 *   alert-tinted border, no pulse — wrong moment to demand attention with
 *   motion.
 * - `null`: idle.
 *
 * Pendency wins over freshness: a `pendingApproval` anywhere in the
 * history overrides a more recent warn/error system message so the user
 * never misses a blocking gate.
 */
export type DockSignal = 'active' | 'attention' | 'error' | null

export function getDockSignal(
  active: Conversation | null | undefined,
): DockSignal {
  if (!active || active.messages.length === 0) return null

  for (const m of active.messages) {
    if (m.role === 'function-trigger' && m.pendingApproval) return 'active'
  }

  const last = active.messages[active.messages.length - 1]
  if (!last) return null

  if (last.role === 'assistant' && last.streaming) return 'active'
  if (last.role === 'thought' && last.streaming) return 'active'
  if (last.role === 'function-trigger' && last.running) return 'active'
  if (last.role === 'system') {
    if (last.tone === 'error') return 'error'
    if (last.tone === 'warn') return 'attention'
  }
  return null
}
