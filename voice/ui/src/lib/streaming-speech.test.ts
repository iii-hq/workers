import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { speechChunks, StreamingSpeech, type SpeechSnapshot } from './streaming-speech'
import { subscribeAutoReplies } from './voice-chat'
import type { ExtensionIii } from './types'

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())
const snapshot = (text: string, revision = 1, entry_id = 'entry'): SpeechSnapshot => ({
  session_id: 'chat', entry_id, revision, timestamp: revision + 10, origin: { turn_id: 'turn' },
  message: { role: 'assistant', content: [{ type: 'text', text }] },
})
const advance = () => vi.advanceTimersByTimeAsync(130)
function rig() {
  const onChunk = vi.fn(), onError = vi.fn()
  const prepare = vi.fn(async (text: string, _complete: boolean) => ({ text, max_chunk_chars: 600 }))
  const speech = new StreamingSpeech({ prepare, onChunk, onError })
  return { speech, prepare, onChunk, onError }
}

describe('streaming phrases', () => {
  it('speaks a complete first sentence while later text is still arriving', async () => {
    const r = rig()
    r.speech.update(snapshot('Olá. A resposta ainda está'))
    await advance()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Olá.')
    expect(r.prepare).toHaveBeenCalledWith('Olá. A resposta ainda está', false)
    r.speech.update(snapshot('Olá. A resposta ainda está chegando. Próxima', 2))
    await advance()
    expect(r.onChunk.mock.calls).toEqual([['Olá.'], ['A resposta ainda está chegando.']])
    r.speech.finish({ id: 'entry', text: 'Olá. A resposta ainda está chegando. Próxima' })
    await advance()
    expect(r.onChunk.mock.calls.at(-1)).toEqual(['Próxima'])
    expect(r.onChunk).toHaveBeenCalledTimes(3)
  })
  it('coalesces token bursts and ignores duplicate/out-of-order revisions', async () => {
    const r = rig()
    r.speech.update(snapshot('Oi. ', 1))
    r.speech.update(snapshot('Oi. Mais. ', 3))
    r.speech.update(snapshot('Oi. Errado. ', 2))
    r.speech.update(snapshot('Oi. Mais. ', 3))
    await advance()
    expect(r.prepare).toHaveBeenCalledOnce()
    expect(r.onChunk.mock.calls).toEqual([['Oi.'], ['Mais.']])
  })
  it('flushes a previous message when the next message begins', async () => {
    const r = rig()
    r.speech.update(snapshot('Primeiro sem ponto'))
    await advance()
    expect(r.onChunk).not.toHaveBeenCalled()
    r.speech.update(snapshot('Segundo. ', 2, 'next'))
    await advance()
    expect(r.onChunk.mock.calls).toEqual([['Primeiro sem ponto'], ['Segundo.']])
  })
  it('never speaks thinking, function arguments or user text', async () => {
    const r = rig()
    r.speech.update({ ...snapshot(''), message: { role: 'assistant', content: [
      { type: 'thinking', text: 'private. ' }, { type: 'function_call', text: 'secret. ' }, { type: 'text', text: 'Visible. ' },
    ] } })
    r.speech.update({ ...snapshot('User. ', 2, 'user'), message: { role: 'user', content: [{ type: 'text', text: 'User. ' }] } })
    await advance()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Visible.')
  })
  it('stops rather than replaying a changed spoken prefix', async () => {
    const r = rig()
    r.speech.update(snapshot('Original. ')); await advance()
    r.speech.update(snapshot('Rewritten. ', 2)); await advance()
    expect(r.onError).toHaveBeenCalledOnce()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Original.')
  })
  it('ignores a preparation result after Stop', async () => {
    const r = rig()
    let resolve!: (value: { text: string; max_chunk_chars: number }) => void
    r.prepare.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.speech.update(snapshot('Old. ')); await advance()
    r.speech.stop()
    resolve({ text: 'Old. ', max_chunk_chars: 600 }); await advance()
    expect(r.onChunk).not.toHaveBeenCalled()
  })
  it('skips stale preparation if a newer snapshot arrived in flight', async () => {
    const r = rig()
    let resolve!: (value: { text: string; max_chunk_chars: number }) => void
    r.prepare.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.speech.update(snapshot('Old. ')); await advance()
    r.speech.update(snapshot('Corrected. ', 2))
    resolve({ text: 'Old. ', max_chunk_chars: 600 }); await advance()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Corrected.')
  })
  it('makes progress even while snapshots keep appending during preparation', async () => {
    const r = rig()
    let resolve!: (value: { text: string; max_chunk_chars: number }) => void
    r.prepare.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.speech.update(snapshot('First. More')); await advance()
    r.speech.update(snapshot('First. More text still arriving', 2))
    resolve({ text: 'First. More', max_chunk_chars: 600 }); await advance()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('First.')
  })
  it('bounds long phrases in Unicode characters, not UTF-16 code units', () => {
    const result = speechChunks('Olá 😀 mundo sem ponto no final', true, 8)
    expect(result.chunks.every((chunk) => [...chunk].length <= 8)).toBe(true)
    expect(result.consumed).toBe('Olá 😀 mundo sem ponto no final')
    expect(speechChunks('Ainda incompleto', false, 600).chunks).toEqual([])
    expect(speechChunks('3.14 é pi. Próxima', false, 600).chunks).toEqual(['3.14 é pi.'])
  })
})

function subscriptionRig() {
  const handlers = new Map<string, (event: any) => void>()
  const on = vi.fn((id: string, handler: (event: any) => void) => { handlers.set(id, handler); return vi.fn() })
  const registerTrigger = vi.fn(() => vi.fn())
  const trigger = vi.fn(async () => ({ session_id: 'chat', turn_id: 'turn', status: 'running' }))
  const onChunk = vi.fn(), onStarted = vi.fn(), onReply = vi.fn(), onError = vi.fn()
  const readReply = vi.fn(async () => ({ id: 'entry', text: 'Primeira. Segunda sem ponto' }))
  const off = subscribeAutoReplies({ on, registerTrigger, trigger, browserId: 'a' } as unknown as ExtensionIii, 'chat', {
    readReply, onStarted, onReply, onError,
    streaming: { prepare: async (text) => ({ text, max_chunk_chars: 600 }), onChunk },
  })
  const emit = (kind: string, event: any) => handlers.get(`iii::voice-ui::${kind}::chat`)!(event)
  const started = (turn_id = 'turn', timestamp = 5) => emit('auto-started', { session_id: 'chat', turn_id, timestamp })
  const completed = (patch = {}) => emit('auto-completed', { session_id: 'chat', turn_id: 'turn', timestamp: 30, status: 'completed', ...patch })
  return { emit, started, completed, off, onChunk, onStarted, onReply, onError, trigger, readReply, registerTrigger }
}

describe('streaming event integration', () => {
  it.each(['failed', 'cancelled'])('stops old speech for a confirmed tied %s turn without a start event', async (status) => {
    const r = subscriptionRig()
    r.started('old', 30)
    r.emit('message-updated', { ...snapshot('Old audio. '), origin: { turn_id: 'old' } })
    r.trigger.mockResolvedValueOnce({ session_id: 'chat', turn_id: 'turn', status })
    r.completed({ timestamp: 30, status }); await advance()
    expect(r.onStarted).toHaveBeenCalledTimes(2)
    expect(r.onChunk).not.toHaveBeenCalled()
    expect(r.readReply).not.toHaveBeenCalled()
    r.completed({ timestamp: 30, status }); await advance()
    expect(r.onStarted).toHaveBeenCalledTimes(2)
    r.off()
  })
  it('invalidates an old reply lookup when a newer cancelled turn has no start event', async () => {
    const r = subscriptionRig()
    let resolve!: (reply: { id: string; text: string }) => void
    r.readReply.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.completed({ turn_id: 'old', timestamp: 30 })
    r.onStarted.mockClear()
    r.trigger.mockResolvedValueOnce({ session_id: 'chat', turn_id: 'turn', status: 'cancelled' })
    r.completed({ timestamp: 40, status: 'cancelled' }); await advance()
    resolve({ id: 'old-reply', text: 'Obsolete audio.' }); await advance()
    expect(r.onReply).not.toHaveBeenCalled()
    expect(r.onChunk).not.toHaveBeenCalled()
    expect(r.onStarted).toHaveBeenCalledOnce()
    r.off()
  })
  it('does not stop a current turn for a nonmatching terminal status lookup', async () => {
    const r = subscriptionRig()
    r.started('current', 30)
    r.trigger.mockResolvedValueOnce({ session_id: 'chat', turn_id: 'current', status: 'running' })
    r.completed({ timestamp: 30, status: 'cancelled' }); await advance()
    expect(r.onStarted).toHaveBeenCalledOnce()
    r.off()
  })
  it('ignores a late confirmed cancellation after a newer turn starts', async () => {
    const r = subscriptionRig()
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.started('old', 30)
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.completed({ timestamp: 30, status: 'cancelled' }); await advance()
    r.started('newest', 50)
    resolve({ session_id: 'chat', turn_id: 'turn', status: 'cancelled' }); await advance()
    expect(r.onStarted).toHaveBeenCalledTimes(2)
    r.off()
  })
  it('retains a terminal event if its own start arrives during confirmation', async () => {
    const r = subscriptionRig()
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.started('old', 20)
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.completed({ timestamp: 30, status: 'cancelled' }); await advance()
    r.started('turn', 25)
    r.emit('message-updated', snapshot('Must not play. '))
    resolve({ session_id: 'chat', turn_id: 'turn', status: 'cancelled' }); await advance()
    expect(r.onStarted).toHaveBeenCalledTimes(3)
    expect(r.onChunk).not.toHaveBeenCalled()
    r.off()
  })
  it('retains every entry and its newest revision while the current-turn lookup is pending', async () => {
    const r = subscriptionRig()
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit('message-added', snapshot('First incomplete', 1, 'first'))
    r.emit('message-updated', { ...snapshot('Second. ', 1, 'second'), timestamp: 20 })
    r.emit('message-updated', snapshot('First latest. ', 3, 'first'))
    r.emit('message-updated', snapshot('First stale. ', 2, 'first'))
    r.emit('message-updated', { ...snapshot('Other turn. ', 1, 'other'), timestamp: 40, origin: { turn_id: 'other' } })
    resolve({ session_id: 'chat', turn_id: 'turn', status: 'running' }); await advance()
    expect(r.onChunk.mock.calls).toEqual([['First latest.'], ['Second.']])
    expect(r.trigger).toHaveBeenCalledOnce()
    r.started('other', 40); await advance()
    expect(r.onChunk.mock.calls.at(-1)).toEqual(['Other turn.'])
    expect(r.onStarted).toHaveBeenCalledTimes(2)
    r.off()
  })
  it('clears buffered entries and ignores the pending status response after unsubscribe', async () => {
    const r = subscriptionRig()
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit('message-added', snapshot('First. ', 1, 'first'))
    r.emit('message-added', snapshot('Second. ', 2, 'second'))
    r.off()
    resolve({ session_id: 'chat', turn_id: 'turn', status: 'running' }); await advance()
    expect(r.onChunk).not.toHaveBeenCalled()
    expect(r.onStarted).not.toHaveBeenCalled()
  })
  it('fails visibly when the reconciliation buffer reaches its entry cap', async () => {
    const r = subscriptionRig()
    r.trigger.mockReturnValueOnce(new Promise(() => {}))
    for (let i = 0; i < 129; i++) r.emit('message-added', snapshot('Pending. ', 1, `entry-${i}`))
    await advance()
    expect(r.onError).toHaveBeenCalledOnce()
    expect(r.onChunk).not.toHaveBeenCalled()
    r.off()
  })
  it('drains all buffered entries if completion overtakes the status lookup', async () => {
    const r = subscriptionRig()
    let resolve!: (current: { session_id: string; turn_id: string; status: string }) => void
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit('message-added', snapshot('Progress without punctuation', 1, 'progress'))
    r.emit('message-updated', { ...snapshot('Primeira. Segunda', 1), timestamp: 20 })
    r.completed(); await advance()
    resolve({ session_id: 'chat', turn_id: 'turn', status: 'completed' }); await advance()
    expect(r.onChunk.mock.calls).toEqual([['Progress without punctuation'], ['Primeira.'], ['Segunda sem ponto']])
    r.off()
  })
  it('starts before completion and only flushes the remaining text at the end', async () => {
    const r = subscriptionRig()
    r.started()
    r.emit('message-updated', snapshot('Primeira. Segunda'))
    await advance()
    expect(r.readReply).not.toHaveBeenCalled()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Primeira.')
    r.completed(); await advance()
    expect(r.onChunk.mock.calls).toEqual([['Primeira.'], ['Segunda sem ponto']])
    r.completed(); await advance()
    expect(r.onChunk).toHaveBeenCalledTimes(2)
    expect(r.registerTrigger).toHaveBeenCalledWith({ type: 'session::message-updated',
      function_id: 'iii::voice-ui::message-updated::chat::a', config: { session_id: 'chat', roles: ['assistant'] } })
    r.off()
  })
  it('reconciles enabling mid-turn without fetching chat history', async () => {
    const r = subscriptionRig()
    r.emit('message-updated', snapshot('Primeira. Ainda'))
    await advance()
    expect(r.trigger).toHaveBeenCalledWith('harness::status', { session_id: 'chat' })
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Primeira.')
    expect(r.readReply).not.toHaveBeenCalled()
    r.off()
  })
  it.each(['cancel', 'new-turn', 'unsubscribe'])('%s discards pending chunks', async (action) => {
    const r = subscriptionRig()
    r.started(); r.emit('message-updated', snapshot('Never. '))
    if (action === 'cancel') r.completed({ status: 'cancelled' })
    if (action === 'new-turn') r.started('new', 40)
    if (action === 'unsubscribe') r.off()
    await advance()
    expect(r.onChunk).not.toHaveBeenCalled()
    r.off()
  })
  it('flushes a completed turn even when the session still owns a future wake', async () => {
    const r = subscriptionRig()
    r.started(); r.emit('message-updated', snapshot('Primeira. Segunda'))
    await advance()
    r.completed({ terminal: false }); await advance()
    expect(r.onChunk.mock.calls).toEqual([['Primeira.'], ['Segunda sem ponto']])
    r.off()
  })
  it('retains a snapshot when a start arrives during current-turn reconciliation', async () => {
    const r = subscriptionRig()
    let resolve!: (value: { session_id: string; turn_id: string; status: string }) => void
    r.trigger.mockReturnValueOnce(new Promise((yes) => { resolve = yes }))
    r.emit('message-updated', snapshot('Primeira. Ainda'))
    r.started()
    resolve({ session_id: 'chat', turn_id: 'turn', status: 'running' }); await advance()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Primeira.')
    expect(r.onStarted).toHaveBeenCalledOnce()
    r.off()
  })
  it('flushes buffered visible progress even when no final reply can be found', async () => {
    const r = subscriptionRig()
    r.readReply.mockResolvedValueOnce(null as any)
    r.started(); r.emit('message-updated', snapshot('Um aviso sem pontuação'))
    r.completed(); await advance()
    expect(r.onChunk).toHaveBeenCalledExactlyOnceWith('Um aviso sem pontuação')
    r.off()
  })
  it('does not read another chat or late data from an old turn', async () => {
    const r = subscriptionRig()
    r.started('new', 40)
    r.emit('message-updated', { ...snapshot('Other. '), session_id: 'other' })
    r.emit('message-updated', snapshot('Old. '))
    await advance()
    expect(r.onChunk).not.toHaveBeenCalled()
    r.off()
  })
})
