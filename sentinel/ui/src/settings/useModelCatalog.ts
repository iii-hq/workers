import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { RPC_TIMEOUT_MS } from '../shared'
import { readCatalog } from './catalog.js'
import type { CatalogModel } from './catalog.js'

/** Tab-scoped, so two open consoles do not fight over the handler id. */
const CHANGED_FN = 'sentinel-ui::models-changed'

/**
 * The router's catalog, re-read when the router says it changed.
 *
 * It changes only on operator action — a credential added or cleared, a
 * provider worker coming or going — so this subscribes instead of polling. An
 * unreachable router leaves the list empty and the form still works: the
 * stored model is shown as configured, and typing a raw id stays possible.
 */
export function useModelCatalog(host: Host): { catalog: CatalogModel[]; loading: boolean } {
  const [catalog, setCatalog] = useState<CatalogModel[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let live = true
    const load = () => {
      host.iii
        .trigger('router::models::list', {}, { timeoutMs: RPC_TIMEOUT_MS })
        .then((response) => {
          if (!live) return
          setCatalog(readCatalog(response))
          setLoading(false)
        })
        .catch(() => {
          if (!live) return
          setCatalog([])
          setLoading(false)
        })
    }
    load()

    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined
    try {
      offHandler = host.iii.on(CHANGED_FN, () => load())
      offTrigger = host.iii.registerTrigger({
        type: 'router::models::changed',
        function_id: `${CHANGED_FN}::${host.iii.browserId}`,
        config: {},
      })
    } catch {
      // A console or router that does not offer the fan-out leaves the list
      // as it was read once. Better than a picker that polls forever.
      offTrigger?.()
      offHandler?.()
      offTrigger = undefined
      offHandler = undefined
    }
    return () => {
      live = false
      offTrigger?.()
      offHandler?.()
    }
  }, [host])

  return { catalog, loading }
}
