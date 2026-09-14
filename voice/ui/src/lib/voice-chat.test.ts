import { describe, expect, it, vi } from 'vitest'
import type { ExtensionIii, SessionMessageEntry } from './types'
import { fetchSpokenReply, finalReply, selectedChatText, subscribeAutoReplies, type SpokenReply } from './voice-chat'

function rig() {
  const handlers = new Map<string, (event: unknown) => void>()
  const disposers: ReturnType<typeof vi.fn>[] = []
  const on = vi.fn((id: string, fn: (event: unknown) => void) => {
    handlers.set(id, fn)
    const off = vi.fn(); disposers.push(off); return off
  })
  const registerTrigger = vi.fn(() => { const off = vi.fn(); disposers.push(off); return off })
  const trigger = vi.fn(async (): Promise<{ session_id: string; turn_id: string; status: string } | null> => null)
  const readReply = vi.fn(async (_turnId: string): Promise<SpokenReply | null> => ({ id: 'new', text: 'Answer.' }))
  const onReply = vi.fn(), onStarted = vi.fn(), onError = vi.fn()
  const off = subscribeAutoReplies({ on, registerTrigger, trigger, browserId: 'client-a' } as unknown as ExtensionIii, 'chat-a', {
    initialReplyId: 'old', readReply, onReply, onStarted, onError,
  })
  let timestamp = 100
  const emit = (patch = {}, kind = 'completed') => handlers.get(`iii::voice-ui::auto-${kind}::chat-a`)?.({
    session_id: 'chat-a', turn_id: 'turn-1', timestamp: ++timestamp, status: 'completed', ...patch,
  })
  return { emit, off, trigger, readReply, onReply, onStarted, onError, registerTrigger, disposers }
}
const tick = async () => { for (let i = 0; i < 8; i++) await Promise.resolve() }

describe('opt-in voice chat', () => {
  it('binds only this chat and does not read history when enabled', () => {
    const r = rig()
    expect(r.readReply).not.toHaveBeenCalled()
    expect(r.registerTrigger).toHaveBeenCalledWith({ type: 'harness::turn-completed',
      function_id: 'iii::voice-ui::auto-completed::chat-a::client-a', config: { session_id: 'chat-a' } })
    r.off()
    for (const dispose of r.disposers) expect(dispose).toHaveBeenCalledOnce()
  })
  it('reads a completed reply once and filters other clients, failures and intermediate turns', async () => {
    const r = rig()
    r.emit({ session_id: 'chat-b' })
    r.emit({ status: 'failed' })
    r.emit({ status: 'cancelled' })
    r.emit({ terminal: false })
    expect(r.readReply).not.toHaveBeenCalled()
    r.emit(); r.emit()
    await tick()
    expect(r.readReply).toHaveBeenCalledExactlyOnceWith('turn-1')
    expect(r.onReply).toHaveBeenCalledExactlyOnceWith({ id: 'new', text: 'Answer.' })
  })
  it('does not replay a stale reply even for a different completion event', async () => {
    const r = rig()
    r.readReply.mockResolvedValueOnce({ id: 'old', text: 'Old reply.' })
    r.emit(); await tick()
    expect(r.onReply).not.toHaveBeenCalled()
  })
  it('does not speak if disabled while fetching the response', async () => {
    const r = rig()
    r.emit(); r.off(); await tick()
    expect(r.onReply).not.toHaveBeenCalled()
    r.emit({ turn_id: 'after-disable' }); await tick()
    expect(r.readReply).toHaveBeenCalledOnce()
  })
  it('a new turn stops playback and invalidates a pending old reply', async () => {
    const r = rig()
    r.emit()
    r.emit({ turn_id: 'turn-2' }, 'started')
    await tick()
    expect(r.onStarted).toHaveBeenCalledOnce()
    expect(r.onReply).not.toHaveBeenCalled()
  })
  it('ignores an old completion delivered after the next turn starts', async () => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 10 }, 'started')
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started')
    r.emit({ turn_id: 'old', timestamp: 20 })
    await tick()
    expect(r.readReply).not.toHaveBeenCalled()
    r.emit({ turn_id: 'new', timestamp: 40 })
    await tick()
    expect(r.readReply).toHaveBeenCalledExactlyOnceWith('new')
  })
  it('ignores duplicate and delayed starts without stopping current playback', async () => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 10 }, 'started')
    r.emit({ turn_id: 'old', timestamp: 10 }, 'started')
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started')
    r.emit({ turn_id: 'new', timestamp: 40 })
    await tick()
    r.emit({ turn_id: 'old', timestamp: 10 }, 'started')
    r.emit({ turn_id: 'unseen-old', timestamp: 15 }, 'started')
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started')
    expect(r.onStarted).toHaveBeenCalledTimes(2)
    expect(r.onReply).toHaveBeenCalledOnce()
  })
  it('accepts a newer completed turn before its own delayed start', async () => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 10 }, 'started')
    r.emit({ turn_id: 'new', timestamp: 40 })
    await tick()
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started')
    expect(r.readReply).toHaveBeenCalledExactlyOnceWith('new')
    expect(r.onStarted).toHaveBeenCalledOnce()
  })
  it('supports enabling mid-turn but rejects late starts after completion', async () => {
    const r = rig()
    r.emit({ timestamp: 20 })
    await tick()
    r.emit({ timestamp: 10 }, 'started')
    expect(r.onReply).toHaveBeenCalledOnce()
    expect(r.onStarted).not.toHaveBeenCalled()
  })
  it('rejects malformed timestamps and does not guess the current turn on a tie', async () => {
    const r = rig()
    r.emit({ timestamp: undefined })
    r.emit({ timestamp: NaN }, 'started')
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started')
    r.emit({ turn_id: 'old', timestamp: 30 })
    await tick()
    expect(r.readReply).not.toHaveBeenCalled()
    expect(r.onStarted).toHaveBeenCalledOnce()
  })
  it('stops existing playback for a valid new start with the same timestamp', async () => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 30 }); await tick()
    expect(r.onReply).toHaveBeenCalledOnce()
    r.trigger.mockResolvedValueOnce({ session_id: 'chat-a', turn_id: 'new', status: 'running' })
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    expect(r.trigger).toHaveBeenCalledExactlyOnceWith('harness::status', { session_id: 'chat-a' })
    expect(r.onStarted).toHaveBeenCalledOnce()
  })
  it('invalidates a pending reply when the next start shares its timestamp', async () => {
    const r = rig()
    let resolve!: (reply: SpokenReply) => void
    r.readReply.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit({ turn_id: 'old', timestamp: 30 })
    r.trigger.mockResolvedValueOnce({ session_id: 'chat-a', turn_id: 'new', status: 'running' })
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    resolve({ id: 'late', text: 'Old answer.' }); await tick()
    expect(r.onStarted).toHaveBeenCalledOnce()
    expect(r.onReply).not.toHaveBeenCalled()
  })
  it('does not play an old tied completion delivered after the next start', async () => {
    const r = rig()
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started')
    r.trigger.mockResolvedValueOnce({ session_id: 'chat-a', turn_id: 'new', status: 'running' })
    r.emit({ turn_id: 'old', timestamp: 30 }); await tick()
    expect(r.readReply).not.toHaveBeenCalled()
    expect(r.onStarted).toHaveBeenCalledOnce()
  })
  it('does not stop newer audio for an unseen old start tied with its completion', async () => {
    const r = rig()
    r.emit({ turn_id: 'new', timestamp: 30 }); await tick()
    r.trigger.mockResolvedValueOnce({ session_id: 'chat-a', turn_id: 'new', status: 'completed' })
    r.emit({ turn_id: 'old', timestamp: 30 }, 'started'); await tick()
    expect(r.onReply).toHaveBeenCalledOnce()
    expect(r.onStarted).not.toHaveBeenCalled()
  })
  it('accepts the current tied completion before its delayed start', async () => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 30 }, 'started')
    r.trigger.mockResolvedValueOnce({ session_id: 'chat-a', turn_id: 'new', status: 'completed' })
    r.emit({ turn_id: 'new', timestamp: 30 }); await tick()
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    expect(r.readReply).toHaveBeenCalledExactlyOnceWith('new')
    expect(r.onStarted).toHaveBeenCalledOnce()
  })
  it.each(['newer-event', 'unsubscribe'])('ignores a late tie lookup after %s', async (action) => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 30 }); await tick()
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit({ turn_id: 'tied', timestamp: 30 }, 'started'); await tick()
    if (action === 'unsubscribe') r.off()
    else r.emit({ turn_id: 'newest', timestamp: 40 }, 'started')
    const calls = r.onStarted.mock.calls.length
    resolve({ session_id: 'chat-a', turn_id: 'tied', status: 'running' }); await tick()
    expect(r.onStarted).toHaveBeenCalledTimes(calls)
  })
  it.each(['running', 'completed'])('keeps a tied completion queued when its start lookup returns %s', async (status) => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 30 }); await tick()
    r.onReply.mockClear()
    r.readReply.mockResolvedValueOnce({ id: 'next', text: 'New answer.' })
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    r.emit({ turn_id: 'new', timestamp: 30 })
    resolve({ session_id: 'chat-a', turn_id: 'new', status }); await tick()
    expect(r.onStarted).toHaveBeenCalledOnce()
    expect(r.onReply).toHaveBeenCalledExactlyOnceWith({ id: 'next', text: 'New answer.' })
    expect(r.onStarted.mock.invocationCallOrder[0]).toBeLessThan(r.onReply.mock.invocationCallOrder[0])
    expect(r.trigger).toHaveBeenCalledOnce()
  })
  it.each(['completed', 'cancelled', 'failed'])('accepts a confirmed start whose turn is already %s', async (status) => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 30 }); await tick()
    r.trigger.mockResolvedValueOnce({ session_id: 'chat-a', turn_id: 'new', status })
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    expect(r.onStarted).toHaveBeenCalledOnce()
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    expect(r.onStarted).toHaveBeenCalledOnce()
  })
  it('reports a failed tie lookup rather than guessing event order', async () => {
    const r = rig()
    r.emit({ turn_id: 'old', timestamp: 30 }); await tick()
    r.trigger.mockRejectedValueOnce(new Error('status unavailable'))
    r.emit({ turn_id: 'new', timestamp: 30 }, 'started'); await tick()
    expect(r.onError).toHaveBeenCalledOnce()
    expect(r.onStarted).not.toHaveBeenCalled()
  })
  it('reports message lookup errors without reading an old reply', async () => {
    const r = rig()
    r.readReply.mockRejectedValueOnce(new Error('offline'))
    r.emit(); await tick()
    expect(r.onError).toHaveBeenCalled()
    expect(r.onReply).not.toHaveBeenCalled()
  })
})

const entry = (overrides: Partial<SessionMessageEntry> = {}): SessionMessageEntry => ({
  entry_id: 'reply', origin: { turn_id: 'new-turn' },
  message: { role: 'assistant', content: [{ type: 'text', text: 'Final answer.' }] }, ...overrides,
})
describe('final text lookup', () => {
  it('ignores commentary/tool calls, other turns and elided entries', () => {
    expect(finalReply(entry(), 'new-turn')).toEqual({ id: 'reply', text: 'Final answer.' })
    expect(finalReply(entry(), 'old-turn')).toBeNull()
    expect(finalReply(entry({ elided: true }))).toBeNull()
    expect(finalReply(entry({ message: { role: 'assistant', content: [
      { type: 'text', text: 'Working…' }, { type: 'function_call' },
    ] } }))).toBeNull()
    expect(finalReply(entry({ message: { role: 'user', content: [{ type: 'text', text: 'User' }] } }))).toBeNull()
  })
  it('preserves source Markdown so the worker parses it once before synthesis', () => {
    const text = '**Olá** com *ênfase*.\n\n```js\nconst raw = "**code**"\n```'
    expect(finalReply(entry({ message: { role: 'assistant', content: [{ type: 'text', text }] } })))
      .toEqual({ id: 'reply', text })
  })
  it('starts from the newest page and uses the final response from the completed turn', async () => {
    const trigger = vi.fn().mockResolvedValue({ messages: [entry({ origin: { turn_id: 'old-turn' } }), entry()], has_more: true })
    expect(await fetchSpokenReply({ trigger } as unknown as ExtensionIii, 'chat', 'new-turn'))
      .toEqual({ id: 'reply', text: 'Final answer.' })
    expect(trigger).toHaveBeenCalledExactlyOnceWith('session::messages-tail', {
      session_id: 'chat', limit: 8, include_custom: false, include_image_data: false,
    })
  })
})

function selection(session = 'chat-a', text = 'chosen words', inMessages = true) {
  const root = { getAttribute: () => session }
  const element = { closest: (selector: string) => selector.startsWith('input') ? null
    : selector === '[data-message-list]' ? inMessages ? {} : null : root }
  return { isCollapsed: false, rangeCount: 1,
    anchorNode: { nodeType: 3, parentElement: element }, focusNode: { nodeType: 3, parentElement: element },
    toString: () => text } as unknown as Selection
}
describe('selected passage privacy', () => {
  it('reads only the highlighted passage in this conversation', () => {
    expect(selectedChatText(selection(), 'chat-a')).toBe('chosen words')
  })
  it('ignores another conversation and non-message panels', () => {
    expect(selectedChatText(selection('chat-b'), 'chat-a')).toBe('')
    expect(selectedChatText(selection('chat-a', 'secret', false), 'chat-a')).toBe('')
    expect(selectedChatText(null, 'chat-a')).toBe('')
  })
})
