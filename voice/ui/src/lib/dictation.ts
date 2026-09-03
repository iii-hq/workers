/**
 * One dictation session shared by every surface that shows it: the composer
 * mic, the chat-header pill and the voice page all subscribe to the same
 * `DictationController`. It registers a browser-local handler for
 * `voice::dictation::start`'s `output_function_id`, starts microphone
 * capture, pushes PCM chunks (base64, a monotonic `seq`, at most four
 * `voice::dictation::push` calls in flight, nothing dropped), reduces the
 * incoming `TranscriptEvent`s into `{ status, partial, committed, error? }`
 * and exposes `start()` / `stop()` / `cancel()`. The reducer is exported
 * standalone so its ordering rules are testable without React.
 */

import type { Host } from '@iii-dev/console-ui'
import { useSyncExternalStore } from 'react'
import { type CaptureHandle, startCapture } from './capture'
import { dictationStart, dictationStop } from './client'
import { base64FromInt16, errorMessage } from './format'
import type { TranscriptEvent } from './types'

const LOCAL_FN = 'iii::voice-ui::transcript'
const MAX_INFLIGHT_PUSHES = 4

export type DictationStatus = 'idle' | 'starting' | 'listening' | 'stopping' | 'error'

export interface DictationState {
  status: DictationStatus
  partial: string
  committed: string[]
  error?: string
}

/** Internal reducer state: `lastSeq` guards ordering and is not exposed. */
export interface DictationReduceState extends DictationState {
  lastSeq: number
}

export const initialDictationReduceState: DictationReduceState = {
  status: 'idle',
  partial: '',
  committed: [],
  lastSeq: -1,
}

/** Pure event reducer: a `seq` at or below the last one seen (an
    out-of-order or duplicate delivery; `TranscriptEvent`s are
    at-least-once, unordered) is ignored entirely. */
export function reduceTranscriptEvent(state: DictationReduceState, event: TranscriptEvent): DictationReduceState {
  if (event.seq <= state.lastSeq) return state
  const lastSeq = event.seq
  switch (event.kind) {
    case 'partial':
      return { ...state, status: 'listening', partial: event.text, lastSeq }
    case 'final':
      return {
        ...state,
        status: 'listening',
        committed: [...state.committed, event.text],
        partial: '',
        lastSeq,
      }
    case 'closed':
      return { ...state, status: 'idle', partial: '', lastSeq }
    case 'error':
      return { ...state, status: 'error', error: event.reason ?? 'dictation error', lastSeq }
    default:
      return { ...state, lastSeq }
  }
}

interface PushQueueItem {
  seq: number
  pcm16Base64: string
}

export class DictationController {
  private state: DictationReduceState = initialDictationReduceState
  private readonly listeners = new Set<() => void>()
  private sessionId: string | null = null
  private capture: CaptureHandle | null = null
  private offHandler: (() => void) | null = null
  private starting = false
  private seq = 0
  private inflight = 0
  private queue: PushQueueItem[] = []

  constructor(private readonly host: Host) {}

  getState = (): DictationReduceState => this.state

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  get listening(): boolean {
    return this.state.status === 'listening' || this.state.status === 'starting'
  }

  private set(next: DictationReduceState | ((s: DictationReduceState) => DictationReduceState)): void {
    this.state = typeof next === 'function' ? next(this.state) : next
    for (const listener of this.listeners) listener()
  }

  private drainQueue(): void {
    const sessionId = this.sessionId
    if (!sessionId) return
    while (this.inflight < MAX_INFLIGHT_PUSHES && this.queue.length > 0) {
      const item = this.queue.shift()
      if (!item) break
      this.inflight += 1
      this.host.iii
        .trigger('voice::dictation::push', {
          session_id: sessionId,
          seq: item.seq,
          pcm16_base64: item.pcm16Base64,
        })
        .catch(() => undefined)
        .finally(() => {
          this.inflight -= 1
          this.drainQueue()
        })
    }
  }

  private dropHandler(): void {
    this.offHandler?.()
    this.offHandler = null
  }

  start = async (): Promise<void> => {
    if (this.starting || this.sessionId) return
    this.starting = true
    this.set((s) => ({ ...s, status: 'starting', error: undefined }))
    this.offHandler = this.host.iii.on<TranscriptEvent>(LOCAL_FN, (event) => {
      this.set((s) => reduceTranscriptEvent(s, event))
    })
    try {
      const res = await dictationStart(this.host.iii, {
        output_function_id: `${LOCAL_FN}::${this.host.iii.browserId}`,
      })
      this.sessionId = res.session_id
      this.seq = 0
      this.queue = []
      try {
        this.capture = await startCapture({
          onChunk: ({ pcm16 }) => {
            const seq = this.seq
            this.seq += 1
            this.queue.push({ seq, pcm16Base64: base64FromInt16(pcm16) })
            this.drainQueue()
          },
        })
        this.set((s) => ({ ...s, status: 'listening' }))
      } catch (captureErr) {
        const sessionId = this.sessionId
        this.sessionId = null
        if (sessionId) {
          dictationStop(this.host.iii, { session_id: sessionId, discard: true }).catch(() => undefined)
        }
        throw captureErr
      }
    } catch (err) {
      this.dropHandler()
      this.set((s) => ({ ...s, status: 'error', error: errorMessage(err) }))
    } finally {
      this.starting = false
    }
  }

  stop = async (): Promise<string> => {
    const sessionId = this.sessionId
    this.capture?.stop()
    this.capture = null
    if (!sessionId) return this.state.committed.join(' ')
    this.sessionId = null
    this.set((s) => ({ ...s, status: 'stopping' }))
    try {
      const res = await dictationStop(this.host.iii, { session_id: sessionId })
      this.dropHandler()
      this.set((s) => ({ ...s, status: 'idle', partial: '' }))
      return res.text
    } catch (err) {
      this.dropHandler()
      this.set((s) => ({ ...s, status: 'error', error: errorMessage(err) }))
      return this.state.committed.join(' ')
    }
  }

  cancel = async (): Promise<void> => {
    const sessionId = this.sessionId
    this.capture?.stop()
    this.capture = null
    this.sessionId = null
    this.dropHandler()
    if (sessionId) {
      await dictationStop(this.host.iii, { session_id: sessionId, discard: true }).catch(() => undefined)
    }
    this.set({ ...initialDictationReduceState })
  }
}

export function createDictationController(host: Host): DictationController {
  return new DictationController(host)
}

export interface UseDictationResult {
  state: DictationState
  start: () => Promise<void>
  stop: () => Promise<string>
  cancel: () => Promise<void>
}

/** Subscribe a component to the shared controller. */
export function useDictation(controller: DictationController): UseDictationResult {
  const state = useSyncExternalStore(controller.subscribe, controller.getState, controller.getState)
  return {
    state: {
      status: state.status,
      partial: state.partial,
      committed: state.committed,
      error: state.error,
    },
    start: controller.start,
    stop: controller.stop,
    cancel: controller.cancel,
  }
}
