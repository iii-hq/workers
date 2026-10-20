/**
 * The tour that waits for an example prompt. Picking an example on Ready
 * starts a chat; the tour opens when that chat's agent has answered or after
 * `TOUR_DELAY_MS`, whichever comes first, so the operator watches the first
 * answer arrive before anything else asks for their attention.
 *
 * The schedule is kept in this browser's storage, so a reload while the
 * agent is still answering keeps the promise: the remaining delay is armed
 * again, and the chat's turn is listened for again.
 *
 * When the answer is what opened the tour, the tour is told about it
 * (`onboarding::steps::observe`): its first step waits on exactly that
 * event, which fired before the tour was listening, and would otherwise
 * ask for a message the operator has already sent.
 */

import { useEffect, useSyncExternalStore } from 'react'
import { getTurnStatus, isTurnActive } from '@/lib/backend/harness-send'
import { subscribeEngineTrigger } from '@/lib/engine-trigger'
import { getIiiClient } from '@/lib/iii-client'
import type { TurnCompletedEvent } from '@/types/iii-agent-event'
import { openTour, prepareTour } from './tour'

const TURN_COMPLETED = 'harness::turn-completed'

/** The longest the tour waits for the example chat's answer. */
export const TOUR_DELAY_MS = 5 * 60_000

const STORAGE_KEY = 'iii.onboarding.tour-schedule'

export interface TourSchedule {
  /** The example chat whose answer opens the tour. */
  sessionId: string
  /** Epoch ms when the tour opens even without an answer. */
  dueAt: number
}

const listeners = new Set<() => void>()
let cached: TourSchedule | null | undefined

function read(): TourSchedule | null {
  if (cached !== undefined) return cached
  cached = null
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    const parsed = raw ? (JSON.parse(raw) as Partial<TourSchedule>) : null
    if (
      parsed &&
      typeof parsed.sessionId === 'string' &&
      typeof parsed.dueAt === 'number'
    ) {
      cached = { sessionId: parsed.sessionId, dueAt: parsed.dueAt }
    }
  } catch {
    // Storage blocked or a malformed entry: no schedule.
  }
  return cached
}

function write(next: TourSchedule | null): void {
  cached = next
  try {
    if (next) window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
    else window.localStorage.removeItem(STORAGE_KEY)
  } catch {
    // Storage blocked: the schedule lives for this page only.
  }
  for (const listener of [...listeners]) listener()
}

/** Open the tour once `sessionId` has answered, or after the delay. */
export function scheduleTour(sessionId: string, now = Date.now()): void {
  write({ sessionId, dueAt: now + TOUR_DELAY_MS })
}

/** Drop a pending tour: it opened, or the operator opened it themselves. */
export function clearTourSchedule(): void {
  if (read()) write(null)
}

export function readTourSchedule(): TourSchedule | null {
  return read()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** A turn that ended for good: a `terminal: false` turn hands over to a later one. */
function endsTheRun(payload: unknown, sessionId: string): boolean {
  const event = payload as Partial<TurnCompletedEvent> | null
  return event?.session_id === sessionId && event.terminal !== false
}

/**
 * Tell the tour the example's turn completed, so the step waiting on it
 * closes with the real event as evidence. Only a completed turn counts: a
 * failed or cancelled one gave the operator no answer to read. A worker
 * that predates the function just leaves the step to its own trigger.
 */
async function reportAnswer(event: TurnCompletedEvent | null): Promise<void> {
  if (event?.status !== 'completed') return
  const client = await getIiiClient()
  await client
    .trigger('onboarding::steps::observe', {
      trigger_type: TURN_COMPLETED,
      payload: event,
    })
    .catch(() => undefined)
}

/**
 * Watch the pending schedule and open the tour when it comes due. Mounted
 * once, beside the wizard host, so it outlives the dialog that set it.
 */
export function useScheduledTour(): void {
  const schedule = useSyncExternalStore(subscribe, read, () => null)
  useEffect(() => {
    if (!schedule) return
    let live = true
    /** `event` is the answer that opened the tour; absent when the delay did. */
    const fire = (event: TurnCompletedEvent | null = null) => {
      if (!live) return
      live = false
      clearTourSchedule()
      void prepareTour()
        .then(() => reportAnswer(event))
        .then(openTour)
        .catch((error: unknown) => {
          // Nobody asked for this tour in the moment, so a failure is not
          // worth a dialog; the palette's `Open onboarding` still works.
          console.warn('[onboarding] the scheduled tour could not open', error)
        })
    }
    const timer = setTimeout(
      () => fire(),
      Math.max(0, schedule.dueAt - Date.now()),
    )
    let unsubscribe: (() => void) | undefined
    void subscribeEngineTrigger(
      TURN_COMPLETED,
      { session_id: schedule.sessionId },
      (payload) => {
        if (endsTheRun(payload, schedule.sessionId)) {
          fire(payload as TurnCompletedEvent)
        }
      },
      { handler: 'iii::console::tour_schedule' },
    )
      .then(async (off) => {
        if (!live) return off()
        unsubscribe = off
        // A turn that ended before the binding landed (a quick failure, a
        // reload after the answer) rang no bell: read the session once.
        const status = await getTurnStatus(
          await getIiiClient(),
          schedule.sessionId,
        ).catch(() => null)
        if (status && !isTurnActive(status.status) && !status.expects_wake) {
          fire({
            session_id: schedule.sessionId,
            turn_id: status.turn_id ?? '',
            status: status.status === 'completed' ? 'completed' : 'failed',
            timestamp: Date.now(),
          })
        }
      })
      // No subscription leaves the delay, which still opens the tour.
      .catch(() => undefined)
    return () => {
      live = false
      clearTimeout(timer)
      unsubscribe?.()
    }
  }, [schedule])
}
