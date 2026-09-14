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
