/**
 * Waiting without polling. A setup step that waits for something — a compose
 * operation to finish, a worker to connect, a configuration entry to appear —
 * subscribes to the trigger types that announce the change, reads the state
 * once, and reads it again only when one of those events arrives:
 *
 *   1. arm the subscriptions (before the work they follow is started, so
 *      nothing is missed),
 *   2. run `start` (kick off the work), then one `check` that covers the
 *      instant before the subscriptions were live,
 *   3. one `check` per relevant event (serialized; a burst coalesces),
 *   4. at most one `check` after `silenceMs` without a relevant event — the
 *      safety net for a missed delivery, re-armed only by the next event,
 *   5. `onTimeout` at the overall deadline.
 *
 * Nothing here re-reads on a timer: a quiet system costs no requests.
 */

import { subscribeEngineTrigger } from '@/lib/engine-trigger'

export interface WakeTrigger {
  /** Trigger type, e.g. `compose-operation` or `engine::workers-available`. */
  type: string
  config?: Record<string, unknown>
}

export type WaitCause =
  | { kind: 'start' }
  | { kind: 'event'; type: string; payload: unknown }
  | { kind: 'silence' }

/** A finished wait. Wrapped so `undefined` can be a result. */
export interface Done<T> {
  value: T
}

/**
 * What an event means: `check` reads the state again, `progress` only proves
 * the work is alive (it re-arms the silence check), `ignore` is someone
 * else's event.
 */
export type EventVerdict = 'check' | 'progress' | 'ignore'

export interface WaitOptions<T> {
  /** Base for the browser-local handler ids (`iii::console::<what>`). */
  handler: string
  /** Subscriptions armed before `start` and the first check. */
  triggers: readonly WakeTrigger[]
  /**
   * Runs once the triggers are armed: start the work being followed. `arm`
   * adds a subscription (an operation id known only once started).
   */
  start?: (arm: (trigger: WakeTrigger) => Promise<void>) => Promise<void>
  /** Classify one event; synchronous. Defaults to `check`. A throw fails the wait. */
  onEvent?: (payload: unknown, type: string) => EventVerdict
  /** Read the state once: `{ value }` finishes, `null` keeps waiting, a throw fails. */
  check: (cause: WaitCause) => Promise<Done<T> | null>
  timeoutMs: number
  /** Called once at the deadline: its value resolves the wait, a throw fails it. */
  onTimeout: () => T | Promise<T>
  /** One extra check after this long without a relevant event. */
  silenceMs?: number
  /** A run the wizard cancelled ends the wait with `cancelled` straight away. */
  signal?: { cancelled: boolean }
}

export const CANCELLED_MESSAGE = 'cancelled'

export function waitForEvents<T>(options: WaitOptions<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const disposers: (() => void)[] = []
    let settled = false
    let checking = false
    let queued: WaitCause | null = null
    let silence: ReturnType<typeof setTimeout> | undefined

    const finish = (outcome: { value: T } | { error: unknown }) => {
      if (settled) return
      settled = true
      clearTimeout(silence)
      clearTimeout(deadline)
      for (const dispose of disposers.splice(0)) {
        try {
          dispose()
        } catch {
          // A disposer that throws must not keep the others bound.
        }
      }
      if ('value' in outcome) resolve(outcome.value)
      else reject(outcome.error)
    }
    const cancelled = () => finish({ error: new Error(CANCELLED_MESSAGE) })

    const runCheck = async (cause: WaitCause): Promise<void> => {
      if (settled) return
      if (options.signal?.cancelled) return cancelled()
      if (checking) {
        // One read is in flight: remember one more, an event over a silence.
        if (!queued || cause.kind === 'event') queued = cause
        return
      }
      checking = true
      try {
        const done = await options.check(cause)
        if (done) finish(done)
      } catch (error) {
        finish({ error })
      } finally {
        checking = false
      }
      if (settled || !queued) return
      const next = queued
      queued = null
      await runCheck(next)
    }

    const armSilence = () => {
      if (!options.silenceMs || settled) return
      clearTimeout(silence)
      silence = setTimeout(() => {
        void runCheck({ kind: 'silence' })
      }, options.silenceMs)
    }

    const arm = async (trigger: WakeTrigger): Promise<void> => {
      if (settled) return
      try {
        const off = await subscribeEngineTrigger(
          trigger.type,
          trigger.config ?? {},
          (payload) => {
            if (settled) return
            if (options.signal?.cancelled) return cancelled()
            let verdict: EventVerdict
            try {
              verdict = options.onEvent?.(payload, trigger.type) ?? 'check'
            } catch (error) {
              finish({ error })
              return
            }
            if (verdict === 'ignore') return
            armSilence()
            if (verdict === 'check') {
              void runCheck({ kind: 'event', type: trigger.type, payload })
            }
          },
          { handler: options.handler },
        )
        if (settled) off()
        else disposers.push(off)
      } catch {
        // No live subscription (an engine without this trigger type): the
        // start check and the silence check still run.
      }
    }

    const deadline = setTimeout(() => {
      if (settled) return
      Promise.resolve()
        .then(options.onTimeout)
        .then(
          (value) => finish({ value }),
          (error: unknown) => finish({ error }),
        )
    }, options.timeoutMs)
    disposers.push(onCancel(options.signal, cancelled))

    void (async () => {
      await Promise.all(options.triggers.map(arm))
      try {
        await options.start?.(arm)
      } catch (error) {
        finish({ error })
        return
      }
      armSilence()
      await runCheck({ kind: 'start' })
    })()
  })
}

const cancelListeners = new WeakMap<object, Set<() => void>>()

/**
 * Call `listener` when `signal.cancelled` turns true — the wizard's cancel
 * token is a plain `{ cancelled }` object, so the flag becomes an accessor
 * while someone listens and a plain property again afterwards. Returns the
 * unsubscribe. A token that cannot be wrapped (frozen) is still honoured at
 * the next event or check.
 */
export function onCancel(
  signal: { cancelled: boolean } | undefined,
  listener: () => void,
): () => void {
  if (!signal) return () => undefined
  if (signal.cancelled) {
    queueMicrotask(listener)
    return () => undefined
  }
  let listeners = cancelListeners.get(signal)
  if (!listeners) {
    const set = new Set<() => void>()
    let value: boolean = signal.cancelled
    try {
      Object.defineProperty(signal, 'cancelled', {
        configurable: true,
        enumerable: true,
        get: () => value,
        set: (next: boolean) => {
          value = next
          if (next) for (const fn of [...set]) fn()
        },
      })
    } catch {
      return () => undefined
    }
    cancelListeners.set(signal, set)
    listeners = set
  }
  const set = listeners
  set.add(listener)
  return () => {
    if (!set.delete(listener) || set.size > 0) return
    cancelListeners.delete(signal)
    const value = signal.cancelled
    Object.defineProperty(signal, 'cancelled', {
      configurable: true,
      enumerable: true,
      writable: true,
      value,
    })
  }
}
