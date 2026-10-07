import type { Host } from '@iii-dev/console-ui'
import { useEffect } from 'react'
import type { CompletedEvent } from './types'

const COMPLETED_FN = 'iii::eval-ui::completed'

type Listener = (event: CompletedEvent) => void

// One `eval::completed` binding per tab, shared by every mounted page: two
// pages calling `host.iii.on` with the same id would fight over it.
const listeners = new Set<Listener>()
let teardown: (() => void) | null = null

function subscribe(host: Host, listener: Listener): () => void {
  listeners.add(listener)
  if (!teardown) {
    const offHandler = host.iii.on<CompletedEvent>(COMPLETED_FN, (event) => {
      for (const current of listeners) current(event)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'eval::completed',
      function_id: `${COMPLETED_FN}::${host.iii.browserId}`,
      config: {},
    })
    teardown = () => {
      offTrigger()
      offHandler()
    }
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && teardown) {
      teardown()
      teardown = null
    }
  }
}

/** Calls `onCompleted` whenever an analysis reaches a terminal status. */
export function useAnalysisCompleted(host: Host, onCompleted: Listener) {
  useEffect(() => subscribe(host, onCompleted), [host, onCompleted])
}
