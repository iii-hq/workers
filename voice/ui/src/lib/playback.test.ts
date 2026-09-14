import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BrowserPlayback } from './playback'
import type { SpeakResponse } from './types'

class FakeAudio {
  static instances: FakeAudio[] = []
  static nextPlay: Promise<void> | undefined
  onended: (() => void) | null = null
  onerror: (() => void) | null = null
  pause = vi.fn()
  removeAttribute = vi.fn()
  load = vi.fn()
  play = vi.fn(() => FakeAudio.nextPlay ?? Promise.resolve())
  constructor(readonly src: string) { FakeAudio.instances.push(this) }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const response: SpeakResponse = {
  backend: 'host', speech_id: 'speech-one', played: false, mime: 'audio/wav', audio_base64: 'UklGRg==',
}
beforeEach(() => {
  FakeAudio.instances = []
  FakeAudio.nextPlay = undefined
  vi.stubGlobal('Audio', FakeAudio)
})
afterEach(() => vi.unstubAllGlobals())

describe('browser-owned read aloud', () => {
  const tick = async () => { for (let i = 0; i < 15; i++) await Promise.resolve() }
  it('queues audio without interrupting and prefetches only one clip', async () => {
    const playback = new BrowserPlayback()
    const first = vi.fn(async () => response)
    const second = vi.fn(async () => response)
    const third = vi.fn(async () => response)
    playback.enqueue(first); playback.enqueue(second); playback.enqueue(third)
    await tick()
    expect(FakeAudio.instances).toHaveLength(1)
    expect(second).toHaveBeenCalledOnce()
    expect(third).not.toHaveBeenCalled()
    expect(FakeAudio.instances[0].pause).not.toHaveBeenCalled()
    FakeAudio.instances[0].onended?.(); await tick()
    expect(FakeAudio.instances).toHaveLength(2)
    expect(third).toHaveBeenCalledOnce()
    FakeAudio.instances[1].onended?.(); await tick()
    expect(FakeAudio.instances).toHaveLength(3)
    FakeAudio.instances[2].onended?.()
    expect(playback.getState().phase).toBe('idle')
  })
  it('Stop discards prefetched audio and never synthesizes the remaining queue', async () => {
    const playback = new BrowserPlayback()
    const pending = deferred<SpeakResponse>()
    const third = vi.fn(async () => response)
    playback.enqueue(async () => response)
    playback.enqueue(() => pending.promise)
    playback.enqueue(third)
    await tick()
    playback.stop(); pending.resolve(response); await tick()
    expect(FakeAudio.instances).toHaveLength(1)
    expect(third).not.toHaveBeenCalled()
    expect(playback.getState().phase).toBe('idle')
  })
  it('does not synthesize a queued callback after an immediate Stop', async () => {
    const playback = new BrowserPlayback()
    const synthesize = vi.fn(async () => response)
    playback.enqueue(synthesize); playback.stop(); await tick()
    expect(synthesize).not.toHaveBeenCalled()
  })
  it('reports a prefetch failure only when that clip reaches the front', async () => {
    const playback = new BrowserPlayback()
    playback.enqueue(async () => response)
    playback.enqueue(async () => { throw new Error('next synthesis failed') })
    await tick()
    expect(playback.getState().phase).toBe('speaking')
    FakeAudio.instances[0].onended?.(); await tick()
    expect(playback.getState()).toEqual({ phase: 'error', message: 'next synthesis failed' })
  })
  it('plays the locally generated WAV only in the requesting browser', async () => {
    const playback = new BrowserPlayback()
    await playback.play(async () => response)
    expect(FakeAudio.instances).toHaveLength(1)
    expect(FakeAudio.instances[0].src).toBe('data:audio/wav;base64,UklGRg==')
    expect(playback.getState()).toEqual({ phase: 'speaking' })
    FakeAudio.instances[0].onended?.()
    expect(playback.getState()).toEqual({ phase: 'idle' })
    expect(FakeAudio.instances[0].removeAttribute).toHaveBeenCalledWith('src')
  })
  it('stopping client A cannot stop or reset client B', async () => {
    const a = new BrowserPlayback()
    const b = new BrowserPlayback()
    await Promise.all([a.play(async () => response), b.play(async () => ({ ...response, speech_id: 'speech-two' }))])
    a.stop()
    expect(FakeAudio.instances[0].pause).toHaveBeenCalledOnce()
    expect(FakeAudio.instances[1].pause).not.toHaveBeenCalled()
    expect(a.getState()).toEqual({ phase: 'idle' })
    expect(b.getState()).toEqual({ phase: 'speaking' })
  })
  it('Stop while preparing ignores a late synthesis response', async () => {
    const playback = new BrowserPlayback()
    const synthesis = deferred<SpeakResponse>()
    const playing = playback.play(() => synthesis.promise)
    expect(playback.getState()).toEqual({ phase: 'loading' })
    playback.stop()
    synthesis.resolve(response)
    await playing
    expect(FakeAudio.instances).toHaveLength(0)
    expect(playback.getState()).toEqual({ phase: 'idle' })
  })
  it('Stop while audio.play is pending cannot reactivate playback', async () => {
    const playback = new BrowserPlayback()
    const started = deferred<void>()
    FakeAudio.nextPlay = started.promise
    const playing = playback.play(async () => response)
    await Promise.resolve()
    playback.stop()
    started.resolve()
    await playing
    expect(FakeAudio.instances[0].pause).toHaveBeenCalledOnce()
    expect(playback.getState()).toEqual({ phase: 'idle' })
  })
  it('an earlier request cannot replace a newer one', async () => {
    const playback = new BrowserPlayback()
    const old = deferred<SpeakResponse>()
    const first = playback.play(() => old.promise)
    await playback.play(async () => ({ ...response, audio_base64: 'bmV3' }))
    old.resolve(response)
    await first
    expect(FakeAudio.instances).toHaveLength(1)
    expect(FakeAudio.instances[0].src).toContain('bmV3')
  })
  it('a stale synthesis error cannot disturb current playback', async () => {
    const playback = new BrowserPlayback()
    const old = deferred<SpeakResponse>()
    const first = playback.play(() => old.promise)
    await playback.play(async () => response)
    old.reject(new Error('late error'))
    await first
    expect(playback.getState()).toEqual({ phase: 'speaking' })
  })
  it('surfaces synthesis and autoplay errors without pretending audio played', async () => {
    const playback = new BrowserPlayback()
    await playback.play(async () => { throw new Error('espeak-ng unavailable') })
    expect(playback.getState()).toEqual({ phase: 'error', message: 'espeak-ng unavailable' })
    FakeAudio.nextPlay = Promise.reject(new Error('autoplay blocked'))
    await playback.play(async () => response)
    expect(playback.getState()).toEqual({ phase: 'error', message: 'autoplay blocked' })
    expect(FakeAudio.instances[0].pause).toHaveBeenCalledOnce()
  })
  it('rejects legacy server playback rather than claiming browser playback', async () => {
    const playback = new BrowserPlayback()
    await playback.play(async () => ({ ...response, audio_base64: undefined, played: true }))
    expect(playback.getState().phase).toBe('error')
    expect(FakeAudio.instances).toHaveLength(0)
  })
  it('handles browser audio errors and allows retry', async () => {
    const playback = new BrowserPlayback()
    await playback.play(async () => response)
    FakeAudio.instances[0].onerror?.()
    expect(playback.getState().phase).toBe('error')
    await playback.play(async () => response)
    expect(playback.getState().phase).toBe('speaking')
  })
})
