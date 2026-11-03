import type { Host } from '@iii-dev/console-ui'
import { useEffect } from 'react'
import type { CompletedEvent } from './types'

const COMPLETED_FN = 'iii::eval-ui::completed'

type Listener = (event: CompletedEvent) => void

// One `eval::on-completed` observer per tab, shared by every mounted page: two
// pages calling `host.iii.on` with the same id would fight over it.
const listeners = new Set<Listener>()
let teardown: (() => void) | null = null

function subscribe(host: Host, listener: Listener): () => void {
  listeners.add(listener)
  if (!teardown) {
    const offHandler = host.iii.on<CompletedEvent>(COMPLETED_FN, (event) => {
      for (const current of listeners) current(event)
    })
    const offObserver = host.iii.observe('eval::on-completed', `${COMPLETED_FN}::${host.iii.browserId}`)
    teardown = () => {
      offObserver()
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
