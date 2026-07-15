import { useEffect, useId, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Page-scoped feed of the memory worker's two trigger types, for surfaces
 * that re-read banks/memories on any change. Same browser-local binding
 * pattern as the worktree lifecycle feed: register a local handler, bind an
 * engine trigger of that type to it, fall back to polling while unbound.
 */

export const MEMORY_EVENT_TRIGGERS = [
  'memory::item-changed',
  'memory::bank-changed',
] as const

const EVENTS_FN = 'console::memory-lifecycle'

export interface UseMemoryEventsOptions {
  enabled: boolean
  /** Fired for EVERY memory event, with its trigger type. */
  onEvent: (triggerType: string, payload: unknown) => void
}

export interface MemoryEventsSubscription {
  /**
   * True once both trigger bindings registered. While false (probe in
   * flight, worker absent, SDK failure) callers fall back to polling.
   */
  bound: boolean
}

export function useMemoryEvents(
  opts: UseMemoryEventsOptions,
): MemoryEventsSubscription {
  const { enabled } = opts
  const onEventRef = useRef(opts.onEvent)
  onEventRef.current = opts.onEvent

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  const [bound, setBound] = useState(false)

  useEffect(() => {
    if (!enabled) {
      setBound(false)
      return
    }
    let cancelled = false
    const offs: Array<() => void> = []

    void (async () => {
      let client: Awaited<ReturnType<typeof getIiiClient>>
      try {
        client = await getIiiClient()
      } catch {
        // Client startup failed; stay unbound so callers fall back to polling.
        return
      }
      if (cancelled) return
      let registered = 0
      for (const triggerType of MEMORY_EVENT_TRIGGERS) {
        const suffix = triggerType.replace(/[^a-zA-Z0-9]/g, '-')
        const localFnId = `${EVENTS_FN}::${suffix}::${instanceId}`
        try {
          offs.push(
            client.on(localFnId, (payload: unknown) => {
              onEventRef.current(triggerType, payload)
            }),
          )
          offs.push(
            client.registerTrigger({
              type: triggerType,
              function_id: `${localFnId}::${client.browserId}`,
              config: {},
            }),
          )
          registered += 1
        } catch {
          // Worker absent or trigger type unregistered; drop the binding.
        }
      }
      if (!cancelled) {
        setBound(registered === MEMORY_EVENT_TRIGGERS.length)
      }
    })()

    return () => {
      cancelled = true
      setBound(false)
      for (const off of offs) off()
    }
  }, [enabled, instanceId])

  return { bound }
}
