/** Audio belongs to the requesting browser, never a global worker playback. */
import { useEffect, useMemo, useSyncExternalStore } from 'react'
import { errorMessage } from './format'
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
    this.releaseAudio()
    this.set({ phase: 'idle' })
  }

  play = async (synthesize: () => Promise<SpeakResponse>): Promise<void> => {
    this.stop()
    const operation = this.operation
    this.set({ phase: 'loading' })
    try {
      const response = await synthesize()
      if (operation !== this.operation) return
      if (!response.audio_base64) throw new Error('The voice worker returned no audio. Update it to enable browser playback.')
      const mime = response.mime ?? 'audio/mpeg'
      if (!/^audio\/[a-z0-9.+-]+$/i.test(mime)) throw new Error('The voice worker returned an unsupported audio type.')
      const audio = new Audio(`data:${mime};base64,${response.audio_base64}`)
      this.audio = audio
      audio.onended = () => {
        if (operation === this.operation) this.stop()
      }
      audio.onerror = () => {
        if (operation !== this.operation) return
        this.stop()
        this.set({ phase: 'error', message: 'Audio playback failed in this browser.' })
      }
      await audio.play()
      // Stop, unmount or onended may have happened while play() was pending.
      if (operation === this.operation) this.set({ phase: 'speaking' })
    } catch (err) {
      if (operation !== this.operation) return
      this.stop()
      this.set({ phase: 'error', message: errorMessage(err) })
    }
  }
}

export function useBrowserPlayback() {
  const playback = useMemo(() => new BrowserPlayback(), [])
  const state = useSyncExternalStore(playback.subscribe, playback.getState, playback.getState)
  useEffect(() => () => playback.stop(), [playback])
  return { state, play: playback.play, stop: playback.stop }
}
