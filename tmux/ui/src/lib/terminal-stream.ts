/**
 * Terminal output arrives as fire-and-forget bus calls, which are NOT
 * ordered: frames carry a per-terminal sequence and only a contiguous run
 * reaches xterm. A gap that outlives `stallMs` asks the caller to replay the
 * missing range from the worker's ring buffer.
 */

import type { TerminalOutputEvent } from './types'

export interface TerminalFrame {
  sequence: number
  data: string
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

/**
 * Hold `frame` until every sequence after `afterSequence` before it has
 * arrived, and return the contiguous run it completes (possibly empty).
 * Frames at or below `afterSequence` are duplicates and drop.
 */
export function bufferTerminalFrame(
  pending: Map<number, TerminalFrame>,
  frame: TerminalFrame,
  afterSequence: number,
): TerminalFrame[] {
  if (!isNonNegativeSafeInteger(afterSequence)) throw new Error('invalid afterSequence')
  if (!isNonNegativeSafeInteger(frame.sequence)) throw new Error('invalid frame sequence')
  if (frame.sequence <= afterSequence) return []
  const existing = pending.get(frame.sequence)
  if (existing && existing.data !== frame.data) {
    throw new Error(`conflicting terminal frame data for sequence ${frame.sequence}`)
  }
  pending.set(frame.sequence, frame)
  const contiguous: TerminalFrame[] = []
  let sequence = afterSequence + 1
  while (pending.has(sequence)) {
    const next = pending.get(sequence) as TerminalFrame
    pending.delete(sequence)
    contiguous.push(next)
    sequence += 1
  }
  return contiguous
}

export interface OrderedWriterOptions {
  /** Paint one frame's base64 payload. */
  write(data: string): void
  /** The tmux client ended; called once nothing sequenced is still pending. */
  onEof(event: TerminalOutputEvent): void
  /** A gap outlived `stallMs`: replay after `lastSequence()`. */
  onStall(): void
  stallMs?: number
  setTimer?: (callback: () => void, ms: number) => unknown
  clearTimer?: (handle: unknown) => void
}

export interface OrderedWriter {
  feed(event: TerminalOutputEvent): void
  /** Frames a replay returned. */
  feedFrames(frames: readonly TerminalFrame[]): void
  /** Skip everything up to `sequence` (a truncated replay before a redraw). */
  skipTo(sequence: number): void
  /** Start over for another terminal. */
  reset(): void
  lastSequence(): number
  pendingCount(): number
  dispose(): void
}

export const STALL_MS = 250

export function createOrderedWriter(options: OrderedWriterOptions): OrderedWriter {
  const stallMs = options.stallMs ?? STALL_MS
  const setTimer = options.setTimer ?? ((callback, ms) => setTimeout(callback, ms))
  const clearTimer = options.clearTimer ?? ((handle) => clearTimeout(handle as ReturnType<typeof setTimeout>))
  let last = 0
  let pending = new Map<number, TerminalFrame>()
  // The eof event carries no sequence, so it waits until every frame in
  // flight has been painted: ending first would drop the final output.
  let eof: TerminalOutputEvent | null = null
  let timer: unknown = null

  const settle = () => {
    if (eof !== null && pending.size === 0) {
      const event = eof
      eof = null
      options.onEof(event)
    }
    if (pending.size > 0) {
      if (timer === null) {
        timer = setTimer(() => {
          timer = null
          if (pending.size > 0) options.onStall()
        }, stallMs)
      }
    } else if (timer !== null) {
      clearTimer(timer)
      timer = null
    }
  }

  const accept = (frame: TerminalFrame) => {
    let run: TerminalFrame[]
    try {
      run = bufferTerminalFrame(pending, frame, last)
    } catch {
      // A conflicting duplicate cannot be ordered; keep the first copy.
      return
    }
    for (const next of run) {
      last = next.sequence
      if (next.data) options.write(next.data)
    }
  }

  return {
    feed(event) {
      if (event.sequence === null || event.sequence === undefined) {
        if (event.eof) eof = event
        settle()
        return
      }
      accept({ sequence: event.sequence, data: event.data ?? '' })
      if (event.eof) eof = event
      settle()
    },
    feedFrames(frames) {
      for (const frame of frames) accept(frame)
      settle()
    },
    skipTo(sequence) {
      if (sequence > last) last = sequence
      for (const key of [...pending.keys()]) if (key <= last) pending.delete(key)
      // What is pending may now continue the skipped-to sequence.
      const run: TerminalFrame[] = []
      while (pending.has(last + 1)) {
        const next = pending.get(last + 1) as TerminalFrame
        pending.delete(last + 1)
        last = next.sequence
        run.push(next)
      }
      for (const next of run) if (next.data) options.write(next.data)
      settle()
    },
    reset() {
      pending = new Map()
      eof = null
      last = 0
      if (timer !== null) {
        clearTimer(timer)
        timer = null
      }
    },
    lastSequence: () => last,
    pendingCount: () => pending.size,
    dispose() {
      if (timer !== null) clearTimer(timer)
      timer = null
      pending.clear()
      eof = null
    },
  }
}
