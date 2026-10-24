import { useEffect, useRef } from 'react'
import { subscribeEngineTrigger } from '@/lib/engine-trigger'

/**
 * Run `onChange` whenever a worker connects, registers its name, or leaves:
 * the engine's `engine::workers-available` trigger bound to a browser-local
 * handler, so the UI reacts the instant a worker is added or removed —
 * whether from an in-app CTA or `iii trigger compose::add` in a terminal. The
 * event names only a worker id, so callers re-read what they show
 * (`engine::workers::list`, a query, …).
 *
 * `subscribeEngineTrigger` gives every binding its own handler id under
 * `fnId`, so two tabs or two mounts never share events. If the binding
 * fails, callers keep what their mount-time read showed.
 */
export interface UseWorkersChangedOptions {
  /** Only subscribe when enabled (e.g. the real backend). */
  enabled: boolean
  /** Base id for the browser-local handler. */
  fnId: string
  onChange: () => void
}

export function useWorkersChanged({
  enabled,
  fnId,
  onChange,
}: UseWorkersChangedOptions): void {
  // Keep the latest callback in a ref so a changing identity never forces a
  // re-subscription (the engine binding is the expensive part).
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let dispose: (() => void) | undefined
    subscribeEngineTrigger(
      'engine::workers-available',
      {},
      () => onChangeRef.current(),
      { handler: fnId },
    )
      .then((off) => {
        if (cancelled) off()
        else dispose = off
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
      dispose?.()
    }
  }, [enabled, fnId])
}
