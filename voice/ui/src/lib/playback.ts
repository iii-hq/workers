/** Audio belongs to the requesting browser, never a global worker playback. */
import { useEffect, useMemo, useSyncExternalStore } from 'react'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { SpeakResponse } from './types'

export type PlaybackState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'speaking' }
  | { phase: 'error'; message: string }

/** One instance per read-aloud control; stopping it cannot stop another client. */
export class BrowserPlayback {
  private state: PlaybackState = { phase: 'idle' }
  private audio: HTMLAudioElement | null = null
  private operation = 0
  private working = false
  private queue: Array<() => Promise<SpeakResponse>> = []
  private next: Promise<{ response: SpeakResponse } | { error: unknown }> | null = null
  private readonly listeners = new Set<() => void>()

  getState = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  private set(state: PlaybackState) {
    this.state = state
    for (const listener of this.listeners) listener()
  }

  private releaseAudio() {
    const audio = this.audio
    this.audio = null
    if (!audio) return
    audio.onended = null
    audio.onerror = null
    audio.pause()
    audio.removeAttribute('src')
    audio.load()
  }

  stop = () => {
    this.operation += 1
    this.queue = []
    this.next = null
    this.working = false
    this.releaseAudio()
    this.set({ phase: 'idle' })
  }

  play = async (synthesize: () => Promise<SpeakResponse>): Promise<void> => {
    this.stop()
    return this.start(synthesize, this.operation)
  }

  /** Append without interrupting the current clip; only one clip is prefetched. */
  enqueue = (synthesize: () => Promise<SpeakResponse>) => {
    if (this.state.phase === 'error') return
    if (this.queue.length >= 128) {
      this.stop()
      this.set({ phase: 'error', message: 'Read-aloud queue is full. Stop and read a shorter passage.' })
      return
    }
    this.queue.push(synthesize)
    if (!this.working) this.advance()
    else if (this.audio) this.prefetch()
  }

  private prefetch() {
    if (!this.next && this.queue.length) {
      const synthesize = this.queue.shift()!
      const operation = this.operation
      this.next = Promise.resolve().then(() => {
        if (operation !== this.operation) throw new Error('Read aloud cancelled.')
        return synthesize()
      }).then(
        (response) => ({ response }), (error: unknown) => ({ error }))
    }
  }

  private advance() {
    this.prefetch()
    const next = this.next
    this.next = null
    if (!next) { this.working = false; this.set({ phase: 'idle' }); return }
    void this.start(async () => {
      const result = await next
      if ('error' in result) throw result.error
      return result.response
    }, this.operation)
  }

  private async start(synthesize: () => Promise<SpeakResponse>, operation: number): Promise<void> {
    this.working = true
    this.set({ phase: 'loading' })
    let audio: HTMLAudioElement | null = null
    try {
      const response = await synthesize()
      if (operation !== this.operation) return
      if (!response.audio_base64) throw new Error('The voice worker returned no audio. Update it to enable browser playback.')
      const mime = response.mime ?? 'audio/mpeg'
      if (!/^audio\/[a-z0-9.+-]+$/i.test(mime)) throw new Error('The voice worker returned an unsupported audio type.')
      audio = new Audio(`data:${mime};base64,${response.audio_base64}`)
      this.audio = audio
      audio.onended = () => {
        if (operation !== this.operation || this.audio !== audio) return
        this.releaseAudio()
        this.advance()
      }
      audio.onerror = () => {
        if (operation !== this.operation || this.audio !== audio) return
        this.stop()
        this.set({ phase: 'error', message: 'Audio playback failed in this browser.' })
      }
      this.prefetch()
      await audio.play()
      // Stop, unmount or onended may have happened while play() was pending.
      if (operation === this.operation && this.audio === audio) this.set({ phase: 'speaking' })
    } catch (err) {
      if (operation !== this.operation || (audio && this.audio !== audio)) return
      this.stop()
      this.set({ phase: 'error', message: errorMessage(err) })
    }
  }
}

export function useBrowserPlayback() {
  const playback = useMemo(() => new BrowserPlayback(), [])
  const state = useSyncExternalStore(playback.subscribe, playback.getState, playback.getState)
  useEffect(() => () => playback.stop(), [playback])
  return { state, play: playback.play, enqueue: playback.enqueue, stop: playback.stop }
}
