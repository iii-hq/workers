import { useEffect, useId, useRef } from 'react'
import { getIiiClient } from '@/lib/iii-client'

export const COMPOSE_CHANGED_TRIGGER = 'compose-ui::changed'

export interface UseComposeChangedOptions {
  enabled: boolean
  fnId: string
  onEvent: () => void
}

export function useComposeChanged(opts: UseComposeChangedOptions): void {
  const { enabled, fnId, onEvent } = opts
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined

    void (async () => {
      const client = await getIiiClient()
      if (cancelled) return
      const localFnId = `${fnId}::${instanceId}`
      try {
        offHandler = client.on(localFnId, () => {
          onEventRef.current()
        })
        offTrigger = client.registerTrigger({
          type: COMPOSE_CHANGED_TRIGGER,
          function_id: `${localFnId}::${client.browserId}`,
          config: {},
        })
      } catch {
        offTrigger?.()
        offHandler?.()
        offTrigger = undefined
        offHandler = undefined
      }
    })()

    return () => {
      cancelled = true
      offTrigger?.()
      offHandler?.()
    }
  }, [enabled, fnId, instanceId])
}
