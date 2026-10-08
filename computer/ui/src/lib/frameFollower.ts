/**
 * Notify-then-fetch follower for one session's newest screencast frame.
 *
 * The worker keeps exactly one frame per session and fires
 * `computer::frame-changed` (a small, image-free notification) after each
 * one; this follower turns those notifications into `computer::frame` reads:
 *
 * - Ordering: frames are ordered by `(epoch, frame_seq)`. A notification or
 *   read that is not newer than what was applied is ignored, so duplicates
 *   and out-of-order deliveries are harmless; a new `epoch` (worker restart,
 *   restored session) supersedes any older one.
 * - Bounded: at most ONE read in flight and ONE dirty flag. A burst of
 *   notifications during a slow read costs at most one more read, and that
 *   read returns the newest frame (intermediate frames are skipped, never
 *   queued).
 * - `cleared` means nothing to fetch; the last painted image stays.
 * - `resync()` is the initial read and the recovery path (reconnect): it
 *   reads whatever is stored now.
 *
 * Pure and framework-free (type-only imports) so it runs under `node --test`.
 */

export interface FrameChangeNote {
  session_id: string
  epoch: number
  frame_seq: number
  change: 'updated' | 'cleared'
}

/** What `computer::frame` answers (the subset this needs). */
export interface StoredFrame {
  frame?: string | null
  mime: string
  width: number
  height: number
  frame_seq: number
  epoch?: number
}

export interface AppliedFrame {
  data: string
  mime: string
  width: number
  height: number
  epoch: number
  frameSeq: number
}

export interface FrameFollowerOptions {
  sessionId: string
  /** `computer::frame` for this session; `sinceFrame` omits an unchanged image. */
  read: (sinceFrame: number | undefined) => Promise<StoredFrame | null>
  apply: (frame: AppliedFrame) => void
  onError?: (error: unknown) => void
}

export interface FrameFollowerStats {
  notifications: number
  skipped: number
  reads: number
  applied: number
}

export interface FrameFollower {
  /** Feed one `computer::frame-changed` payload. Never throws, never awaits. */
  notify(note: FrameChangeNote): void
  /** Read whatever is stored now (initial read, recovery). */
  resync(): Promise<void>
  stop(): void
  readonly stats: FrameFollowerStats
  /** True while no read is running or pending. */
  idle(): boolean
}

/** Parse a delivered notification; null when it is not one. */
export function parseFrameChange(raw: unknown): FrameChangeNote | null {
  if (!raw || typeof raw !== 'object') return null
  const o = raw as Record<string, unknown>
  if (
    typeof o.session_id !== 'string' ||
    typeof o.epoch !== 'number' ||
    typeof o.frame_seq !== 'number' ||
    (o.change !== 'updated' && o.change !== 'cleared')
  ) {
    return null
  }
  return {
    session_id: o.session_id,
    epoch: o.epoch,
    frame_seq: o.frame_seq,
    change: o.change,
  }
}

export function createFrameFollower(opts: FrameFollowerOptions): FrameFollower {
  const stats: FrameFollowerStats = {
    notifications: 0,
    skipped: 0,
    reads: 0,
    applied: 0,
  }
  // Applied revision.
  let epoch = 0
  let seq = 0
  // Epoch of the newest notification: `since_frame` is only meaningful
  // within the epoch the worker is in.
  let notedEpoch = 0
  let dirty = false
  let full = false
  let draining: Promise<void> | null = null
  let stopped = false

  const newer = (e: number, s: number) =>
    e > epoch || (e === epoch && s > seq)

  function drain(): Promise<void> {
    draining ??= (async () => {
      try {
        while (dirty && !stopped) {
          dirty = false
          const since =
            full || seq === 0 || notedEpoch !== epoch ? undefined : seq
          full = false
          stats.reads++
          let res: StoredFrame | null
          try {
            res = await opts.read(since)
          } catch (error) {
            // No retry timer: the next notification (or resync) reads again.
            opts.onError?.(error)
            continue
          }
          if (stopped || !res?.frame) continue
          const e = res.epoch ?? epoch
          if (!newer(e, res.frame_seq)) continue
          epoch = e
          seq = res.frame_seq
          stats.applied++
          opts.apply({
            data: res.frame,
            mime: res.mime,
            width: res.width,
            height: res.height,
            epoch: e,
            frameSeq: res.frame_seq,
          })
        }
      } finally {
        draining = null
      }
    })()
    return draining
  }

  return {
    stats,
    notify(note) {
      if (stopped) return
      stats.notifications++
      if (
        note.session_id !== opts.sessionId ||
        note.change !== 'updated' ||
        !newer(note.epoch, note.frame_seq)
      ) {
        stats.skipped++
        return
      }
      if (note.epoch > notedEpoch) notedEpoch = note.epoch
      dirty = true
      void drain()
    },
    resync() {
      if (stopped) return Promise.resolve()
      dirty = true
      full = true
      return drain()
    },
    stop() {
      stopped = true
      dirty = false
    },
    idle: () => draining === null && !dirty,
  }
}
