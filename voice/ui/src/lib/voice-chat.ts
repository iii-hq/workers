import type { ExtensionIii, SessionMessageEntry } from './types'
import { StreamingSpeech, type PreparedSpeech, type SpeechSnapshot } from './streaming-speech'

export interface SpokenReply { id: string; text: string }
interface TurnEvent {
  session_id: string
  turn_id: string
  timestamp: number
  status?: string
  terminal?: boolean
  result_error?: string
}

export function finalReply(entry: SessionMessageEntry, turnId?: string): SpokenReply | null {
  if (entry.message?.role !== 'assistant' || entry.elided || (turnId && entry.origin?.turn_id !== turnId)) return null
  const content = entry.message.content ?? []
  if (content.some((block) => block.type === 'function_call')) return null
  const text = content.filter((b) => b.type === 'text').map((b) => b.text ?? '').join('\n').trim()
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
  streaming?: {
    prepare: (text: string, complete: boolean) => Promise<PreparedSpeech>
    onChunk: (text: string) => void
  }
}): () => void {
  let active = true
  let generation = 0
  let lastReplyId = options.initialReplyId
  let tieLookup = Promise.resolve()
  const seen = new Set<string>()
  const started = new Set<string>()
  let latest: { id: string; timestamp: number } | null = null
  let stream: { turnId: string; speech: StreamingSpeech } | null = null
  const pendingSnapshots = new Map<string, { snapshot: SpeechSnapshot; firstTimestamp: number }>()
  let checkingSnapshot = false
  const startStream = (turnId: string) => {
    if (!options.streaming || stream?.turnId === turnId) return
    stream?.speech.stop()
    stream = { turnId, speech: new StreamingSpeech({ ...options.streaming, onError: options.onError }) }
  }
  const stopStream = () => { stream?.speech.stop() }
  const remember = (set: Set<string>, id: string) => {
    set.add(id)
    if (set.size > 128) set.delete(set.values().next().value!)
  }
  const outdated = (event: TurnEvent) => !Number.isFinite(event.timestamp)
    || (latest !== null && event.timestamp < latest.timestamp)
  const offs: Array<() => void> = []
  const failed = (event: TurnEvent) => event.status === 'failed' || event.status === 'cancelled' || Boolean(event.result_error)
  const prunePending = () => {
    for (const [key, { snapshot }] of pendingSnapshots) {
      const turnId = snapshot.origin?.turn_id ?? ''
      if (seen.has(turnId) || (latest && turnId !== latest.id && snapshot.timestamp < latest.timestamp)) {
        pendingSnapshots.delete(key)
      }
    }
  }
  const drainPending = (turnId: string) => {
    const entries = [...pendingSnapshots.entries()]
      .filter(([, entry]) => entry.snapshot.origin?.turn_id === turnId)
      .sort((a, b) => a[1].firstTimestamp - b[1].firstTimestamp)
    for (const [key] of entries) pendingSnapshots.delete(key)
    if (!active || !entries.length) return
    startStream(turnId)
    for (const [, entry] of entries) {
      stream?.speech.update({ ...entry.snapshot, timestamp: entry.firstTimestamp })
    }
  }
  const bind = (kind: 'started' | 'completed', handler: (event: TurnEvent, confirmed?: boolean) => void) => {
    const id = `iii::voice-ui::auto-${kind}::${sessionId}`
    offs.push(iii.on<TurnEvent>(id, (event) => {
      if (!active || event?.session_id !== sessionId || !event.turn_id || outdated(event) || seen.has(event.turn_id)) return
      const needsTerminalConfirmation = kind === 'completed' && failed(event) && latest?.id !== event.turn_id
      if (!needsTerminalConfirmation && (!latest || event.timestamp !== latest.timestamp || event.turn_id === latest.id)) {
        handler(event)
        return
      }
      // Resolve tied timestamps and unseen failed/cancelled turns against the
      // current record. Delivery order and random IDs are not ordering keys.
      // These are serialized event-driven reads, not polling.
      tieLookup = tieLookup.then(async () => {
        if (!active || outdated(event) || seen.has(event.turn_id)) return
        if (latest?.id === event.turn_id) { handler(event); return }
        const request = generation
        try {
          const current = await iii.trigger<{ session_id: string; turn_id: string | null; status: string } | null>(
            'harness::status', { session_id: sessionId })
          if (!active || outdated(event) || seen.has(event.turn_id)
            || (generation !== request && latest?.id !== event.turn_id)
            || current?.session_id !== sessionId || current.turn_id !== event.turn_id) return
          // The turn may finish while this read is in flight. Its identity
          // still confirms the start; the handler's seen/generation guards
          // prevent stopping audio if that completion was already handled.
          if (kind === 'started' || (current.status === event.status
            && ['completed', 'failed', 'cancelled'].includes(current.status))) handler(event, true)
        } catch (error) {
          if (active && generation === request) options.onError(error)
        }
      })
    }))
    offs.push(iii.registerTrigger({ type: `harness::turn-${kind}`,
      function_id: `${id}::${iii.browserId}`, config: { session_id: sessionId } }))
  }
  try {
    bind('started', (event) => {
      if (!active || event?.session_id !== sessionId || !event.turn_id || outdated(event)
        || started.has(event.turn_id) || seen.has(event.turn_id)) return
      remember(started, event.turn_id)
      latest = { id: event.turn_id, timestamp: event.timestamp }
      generation += 1
      if (stream?.turnId !== event.turn_id) options.onStarted()
      startStream(event.turn_id)
      drainPending(event.turn_id)
      prunePending()
    })
    bind('completed', (event, confirmed) => {
      if (!active || event?.session_id !== sessionId || !event.turn_id || seen.has(event.turn_id) || outdated(event)) return
      if (failed(event) && (latest?.id === event.turn_id || confirmed)) {
        latest = { id: event.turn_id, timestamp: event.timestamp }
        generation += 1
        remember(seen, event.turn_id)
        prunePending()
        stream?.speech.stop()
        options.onStarted()
        return
      }
      if (!active || event?.session_id !== sessionId || event.status !== 'completed'
        || (!options.streaming && event.terminal === false) || event.result_error || !event.turn_id || seen.has(event.turn_id)) return
      if (outdated(event)) return
      // A completion from a superseded turn is never allowed to replace the
      // current one, even if transport delivers it after a newer start.
      if (latest && latest.id !== event.turn_id && started.has(event.turn_id)) return
      remember(seen, event.turn_id)
      latest = { id: event.turn_id, timestamp: event.timestamp }
      const request = ++generation
      if (options.streaming) {
        if (stream?.turnId !== event.turn_id) options.onStarted()
        startStream(event.turn_id)
        drainPending(event.turn_id)
      }
      prunePending()
      void options.readReply(event.turn_id).then((reply) => {
        if (!active || generation !== request) return
        if (options.streaming) {
          if (stream?.turnId !== event.turn_id) options.onStarted()
          startStream(event.turn_id)
          stream?.speech.finish(reply?.id === lastReplyId ? null : reply)
        }
        if (!reply?.text || reply.id === lastReplyId) return
        lastReplyId = reply.id
        options.onReply(reply)
      }).catch((error) => {
        if (active && generation === request) options.onError(error)
      })
    })
    if (options.streaming) {
      const accept = (event: SpeechSnapshot) => {
        const turnId = event?.origin?.turn_id
        if (!active || event?.session_id !== sessionId || !turnId || !event.entry_id
          || event.message?.role !== 'assistant' || !Number.isFinite(event.timestamp) || seen.has(turnId)) return
        if (latest?.id === turnId) {
          startStream(turnId)
          drainPending(turnId)
          stream?.speech.update(event)
          return
        }
        if (latest && event.timestamp < latest.timestamp) return
        const revision = event.revision ?? 0
        if (!Number.isSafeInteger(revision) || revision < 0 || event.elided) return
        const key = JSON.stringify([turnId, event.entry_id])
        const previous = pendingSnapshots.get(key)
        prunePending()
        const textSize = (event.message.content ?? []).reduce((size, block) => size + (block.text?.length ?? 0), 0)
        if ((!previous && pendingSnapshots.size >= 128) || textSize > 262144) {
          active = false
          generation += 1
          pendingSnapshots.clear()
          stopStream()
          options.onStarted()
          options.onError(new Error('Streaming read-aloud pending message limit reached.'))
          return
        }
        if (!previous || revision > (previous.snapshot.revision ?? 0)) {
          pendingSnapshots.set(key, { snapshot: event,
            firstTimestamp: Math.min(previous?.firstTimestamp ?? event.timestamp, event.timestamp) })
        }
        if (checkingSnapshot) return
        checkingSnapshot = true
        const request = generation
        void iii.trigger<{ session_id: string; turn_id: string | null } | null>(
          'harness::status', { session_id: sessionId }).then((current) => {
          if (!active) return
          if (generation !== request) {
            if (latest && !seen.has(latest.id)) drainPending(latest.id)
            prunePending()
            return
          }
          if (current?.session_id !== sessionId || !current.turn_id || seen.has(current.turn_id)) return
          const entries = [...pendingSnapshots.values()].filter((entry) => entry.snapshot.origin?.turn_id === current.turn_id)
          if (!entries.length) return
          latest = { id: current.turn_id, timestamp: Math.min(...entries.map((entry) => entry.firstTimestamp)) }
          generation += 1
          options.onStarted()
          startStream(current.turn_id)
          drainPending(current.turn_id)
          prunePending()
        }).catch((error) => {
          if (active && generation === request) options.onError(error)
        }).finally(() => { checkingSnapshot = false })
      }
      for (const kind of ['added', 'updated'] as const) {
        const id = `iii::voice-ui::message-${kind}::${sessionId}`
        offs.push(iii.on<SpeechSnapshot>(id, accept))
        offs.push(iii.registerTrigger({ type: `session::message-${kind}`,
          function_id: `${id}::${iii.browserId}`, config: { session_id: sessionId, roles: ['assistant'] } }))
      }
    }
  } catch (error) {
    active = false
    pendingSnapshots.clear()
    stopStream()
    for (const off of offs.reverse()) off()
    throw error
  }
  return () => {
    active = false
    generation += 1
    pendingSnapshots.clear()
    stream?.speech.stop()
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
