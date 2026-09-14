import type { ExtensionIii, SessionMessageEntry } from './types'
import { stripCodeFences } from './format'

export interface SpokenReply { id: string; text: string }
interface TurnEvent {
  session_id: string
  turn_id: string
  status?: string
  terminal?: boolean
  result_error?: string
}

export function finalReply(entry: SessionMessageEntry, turnId?: string): SpokenReply | null {
  if (entry.message?.role !== 'assistant' || entry.elided || (turnId && entry.origin?.turn_id !== turnId)) return null
  const content = entry.message.content ?? []
  if (content.some((block) => block.type === 'function_call')) return null
  const text = stripCodeFences(content.filter((b) => b.type === 'text').map((b) => b.text ?? '').join('\n')).trim()
  return text ? { id: entry.entry_id, text } : null
}

/** Start at the newest messages, not the first 2,000 messages of a long chat. */
export async function fetchSpokenReply(iii: ExtensionIii, sessionId: string, turnId?: string): Promise<SpokenReply | null> {
  let before: string | undefined
  for (let page = 0; page < 4; page++) {
    const res = await iii.trigger<{ messages: SessionMessageEntry[]; has_more: boolean; oldest_entry_id?: string }>(
      'session::messages-tail', { session_id: sessionId, limit: 8, include_custom: false,
        include_image_data: false, ...(before ? { before_entry_id: before } : {}) })
    for (const entry of [...res.messages].reverse()) {
      const reply = finalReply(entry, turnId)
      if (reply) return reply
    }
    if (!res.has_more || !res.oldest_entry_id || res.oldest_entry_id === before) return null
    before = res.oldest_entry_id
  }
  return null
}

/** Opt-in, future turn events only; no historical reply playback on enable. */
export function subscribeAutoReplies(iii: ExtensionIii, sessionId: string, options: {
  initialReplyId?: string
  readReply: (turnId: string) => Promise<SpokenReply | null>
  onReply: (reply: SpokenReply) => void
  onStarted: () => void
  onError: (error: unknown) => void
}): () => void {
  let active = true
  let generation = 0
  let lastReplyId = options.initialReplyId
  const seen = new Set<string>()
  const offs: Array<() => void> = []
  const bind = (kind: 'started' | 'completed', handler: (event: TurnEvent) => void) => {
    const id = `iii::voice-ui::auto-${kind}::${sessionId}`
    offs.push(iii.on<TurnEvent>(id, handler))
    offs.push(iii.registerTrigger({ type: `harness::turn-${kind}`,
      function_id: `${id}::${iii.browserId}`, config: { session_id: sessionId } }))
  }
  try {
    bind('started', (event) => {
      if (!active || event?.session_id !== sessionId) return
      generation += 1
      options.onStarted()
    })
    bind('completed', (event) => {
      if (!active || event?.session_id !== sessionId || event.status !== 'completed'
        || event.terminal === false || event.result_error || !event.turn_id || seen.has(event.turn_id)) return
      seen.add(event.turn_id)
      if (seen.size > 128) seen.delete(seen.values().next().value!)
      const request = ++generation
      void options.readReply(event.turn_id).then((reply) => {
        if (!active || generation !== request || !reply?.text || reply.id === lastReplyId) return
        lastReplyId = reply.id
        options.onReply(reply)
      }).catch((error) => {
        if (active && generation === request) options.onError(error)
      })
    })
  } catch (error) {
    active = false
    for (const off of offs.reverse()) off()
    throw error
  }
  return () => {
    active = false
    generation += 1
    for (const off of offs.reverse()) off()
  }
}

/** Never read another pane's selection, form secrets, or configuration fields. */
export function selectedChatText(selection: Selection | null, sessionId: string): string {
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return ''
  const rootOf = (node: Node | null) => {
    const element = node?.nodeType === 1 ? node as Element : node?.parentElement
    if (!element || element.closest('input, textarea, [contenteditable="true"]')) return null
    if (!element.closest('[data-message-list]')) return null
    const root = element.closest('[data-chat-session-id]')
    return root?.getAttribute('data-chat-session-id') === sessionId ? root : null
  }
  const start = rootOf(selection.anchorNode)
  if (!start || rootOf(selection.focusNode) !== start) return ''
  return selection.toString().trim()
}
