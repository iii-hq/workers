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
const FLUSH_PUSHES_MS = 2000

export type DictationStatus = 'idle' | 'starting' | 'listening' | 'stopping' | 'error'

export interface DictationState {
  status: DictationStatus
  partial: string
  committed: string[]
  /** Segment index of each committed line, a stable key for rendering. */
  committedIds: number[]
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
  committedIds: [],
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
      return { ...state, status: state.status === 'stopping' ? 'stopping' : 'listening', partial: event.text, lastSeq }
    case 'final':
      return {
        ...state,
        status: state.status === 'stopping' ? 'stopping' : 'listening',
        committed: [...state.committed, event.text],
        committedIds: [...state.committedIds, event.segment],
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

interface StartAttempt {
  generation: number
  cancelled: boolean
  discard: boolean
  cancel: () => void
}

export class DictationController {
  private state: DictationReduceState = initialDictationReduceState
  private readonly listeners = new Set<() => void>()
  private sessionId: string | null = null
  private capture: CaptureHandle | null = null
  private releaseScreenWakeLock: (() => void) | null = null
  private offHandler: (() => void) | null = null
  private starting = false
  private stopping = false
  private startPromise: Promise<void> | null = null
  private opening: StartAttempt | null = null
  private stopPromise: Promise<string> | null = null
  private stopMode: { discard: boolean; opening: StartAttempt | null } | null = null
  private generation = 0
  private seq = 0
  private inflight = 0
  private queue: PushQueueItem[] = []
  private drainWaiters: Array<() => void> = []

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
    const active = ['starting', 'listening', 'stopping'].includes(this.state.status)
    if (active) {
      this.releaseScreenWakeLock ??= this.host.screen?.keepAwake() ?? null
    } else {
      this.releaseScreenWakeLock?.()
      this.releaseScreenWakeLock = null
    }
    for (const listener of this.listeners) listener()
  }

  private drainQueue(): void {
    const sessionId = this.sessionId
    const generation = this.generation
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
          // A timed-out push from an old session must not drain the next one's queue.
          if (generation !== this.generation || sessionId !== this.sessionId) return
          this.inflight -= 1
          this.drainQueue()
          this.notifyDrained()
        })
    }
  }

  private notifyDrained(): void {
    if (this.queue.length > 0 || this.inflight > 0) return
    const waiters = this.drainWaiters
    this.drainWaiters = []
    for (const resolve of waiters) resolve()
  }

  /** Resolves once every queued chunk has been pushed, or after `limitMs`. */
  private flushPushes(limitMs: number): Promise<void> {
    this.drainQueue()
    if (this.queue.length === 0 && this.inflight === 0) return Promise.resolve()
    return new Promise((resolve) => {
      const timer = window.setTimeout(resolve, limitMs)
      this.drainWaiters.push(() => {
        window.clearTimeout(timer)
        resolve()
      })
    })
  }

  private dropHandler(): void {
    this.offHandler?.()
    this.offHandler = null
  }

  /** getUserMedia cannot be aborted; invalidate it and release the UI now. */
  private cancelOpening(discard: boolean): void {
    const attempt = this.opening
    if (!attempt) return
    attempt.discard = discard
    attempt.cancelled = true
    attempt.cancel()
    this.opening = null
    this.starting = false
    this.startPromise = null
  }

  /** Detach first, then stop capture: its final flush must not revive a closed session. */
  private releaseSession(): void {
    this.cancelOpening(true)
    const capture = this.capture
    this.capture = null
    this.sessionId = null
    this.queue = []
    this.inflight = 0
    this.dropHandler()
    this.notifyDrained()
    void capture?.stop().catch(() => undefined)
  }

  start = (): Promise<void> => {
    if (this.starting || this.stopping || this.sessionId) return this.startPromise ?? Promise.resolve()
    this.starting = true
    const generation = ++this.generation
    this.seq = 0
    this.queue = []
    this.inflight = 0
    this.set({ ...initialDictationReduceState, status: 'starting' })
    let cancel!: () => void
    const cancelled = new Promise<void>((resolve) => { cancel = resolve })
    const attempt: StartAttempt = { generation, cancelled: false, discard: true, cancel }
    this.opening = attempt
    this.startPromise = Promise.race([this.openSession(attempt), cancelled]).finally(() => {
      // A cancelled acquisition may settle after the next session has started.
      if (this.opening !== attempt) return
      this.opening = null
      this.starting = false
      this.startPromise = null
    })
    return this.startPromise
  }

  private async openSession(attempt: StartAttempt): Promise<void> {
    const { generation } = attempt
    const obsolete = () => attempt.cancelled || generation !== this.generation
    try {
      let active = true
      const off = this.host.iii.on<TranscriptEvent>(LOCAL_FN, (event) => {
        if (!active || obsolete() || event.session_id !== this.sessionId) return
        const next = reduceTranscriptEvent(this.state, event)
        if (next === this.state) return
        if (event.kind === 'closed' || event.kind === 'error') this.releaseSession()
        // Final/closed events may arrive before the Stop response. Keep every
        // surface out of listening until that one shared shutdown has settled.
        this.set(this.stopping && event.kind !== 'error' ? { ...next, status: 'stopping' } : next)
      })
      this.offHandler = () => { active = false; off() }
      const res = await dictationStart(this.host.iii, {
        output_function_id: `${LOCAL_FN}::${this.host.iii.browserId}`,
      })
      if (obsolete()) {
        // Stop can finish before the server responds. Retire only that late session.
        await dictationStop(this.host.iii, attempt.discard
          ? { session_id: res.session_id, discard: true } : { session_id: res.session_id }).catch(() => undefined)
        return
      }
      this.sessionId = res.session_id
      // Stop can be requested while the worker is loading its streaming model.
      if (this.stopping) return
      const capture = await startCapture({
        onChunk: ({ pcm16 }) => {
          if (obsolete() || this.sessionId !== res.session_id) return
          if (this.starting && this.stopping) return
          const seq = this.seq++
          this.queue.push({ seq, pcm16Base64: base64FromInt16(pcm16) })
          this.drainQueue()
        },
      })
      if (obsolete() || this.sessionId !== res.session_id) {
        // Permission may resolve after cancellation, closure or a newer start.
        await capture.stop()
        return
      }
      this.capture = capture
      // A pending Stop owns this newly opened capture and will release it next.
      if (!this.stopping) this.set((s) => ({ ...s, status: 'listening' }))
    } catch (err) {
      if (obsolete()) return
      const sessionId = this.sessionId
      this.releaseSession()
      if (sessionId) {
        void dictationStop(this.host.iii, { session_id: sessionId, discard: true }).catch(() => undefined)
      }
      this.set((s) => ({ ...s, status: 'error', partial: '', error: errorMessage(err) }))
    }
  }

  stop = (): Promise<string> => this.finish(false)

  private finish(discard: boolean): Promise<string> {
    if (this.stopPromise) {
      if (discard && this.stopMode) {
        this.stopMode.discard = true
        if (this.stopMode.opening) this.stopMode.opening.discard = true
      }
      return this.stopPromise
    }
    const mode = { discard, opening: this.opening }
    this.stopMode = mode
    this.stopping = true
    this.cancelOpening(discard)
    this.set((s) => ({ ...s, status: 'stopping', partial: '' }))
    this.stopPromise = this.closeSession(mode).finally(() => {
      this.stopping = false
      this.stopPromise = null
      this.stopMode = null
    })
    return this.stopPromise
  }

  private async closeSession(mode: { discard: boolean }): Promise<string> {
    try {
      // Never wait on microphone permission. A cancelled attempt cleans up its
      // own late session/stream without touching the current controller.
      const sessionId = this.sessionId
      const capture = this.capture
      this.capture = null
      await capture?.stop()
      if (!mode.discard) await this.flushPushes(FLUSH_PUSHES_MS)
      this.queue = []
      // Keep sessionId attached until the response so final events stay scoped.
      const res = sessionId ? await dictationStop(this.host.iii,
        mode.discard ? { session_id: sessionId, discard: true } : { session_id: sessionId }) : null
      this.releaseSession()
      // A later Cancel suppresses the result even if Stop was already sent.
      this.set(mode.discard ? { ...initialDictationReduceState }
        : { ...this.state, status: this.state.error ? 'error' : 'idle', partial: '' })
      return mode.discard ? '' : res?.text ?? this.state.committed.join(' ')
    } catch (err) {
      this.releaseSession()
      this.set((s) => ({ ...(mode.discard ? initialDictationReduceState : s), status: 'error', partial: '', error: errorMessage(err) }))
      return mode.discard ? '' : this.state.committed.join(' ')
    }
  }

  cancel = async (): Promise<void> => {
    await this.finish(true)
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
      committedIds: state.committedIds,
      error: state.error,
    },
    start: controller.start,
    stop: controller.stop,
    cancel: controller.cancel,
  }
}
