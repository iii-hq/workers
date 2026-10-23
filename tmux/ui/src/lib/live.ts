/**
 * The page's picture of tmux: both lists fetched once, then kept current by
 * the worker's `tmux::changed` trigger type, which pushes the whole overview
 * whenever anything in it changes. The binding goes through a tab-scoped
 * handler (`iii::` keeps per-event invocations out of the traces) and is
 * collected with the tab. A transport that comes back fetches again, since
 * pushes sent while it was down are gone.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { TmuxApi } from './api'
import { newerSnapshot } from './model'
import type { Overview, PortsSnapshot, SessionsSnapshot } from './types'

const CHANGED_HANDLER = 'iii::tmux-ui::changed'

export interface TmuxOverview {
  sessions: SessionsSnapshot | null
  ports: PortsSnapshot | null
  /** The first fetch has not answered yet. */
  loading: boolean
  /** Why the last fetch failed; cleared by the next success or push. */
  error: string | null
  /** A `tmux::changed` push has arrived since the page mounted. */
  live: boolean
  refresh(): void
}

function isOverview(value: unknown): value is Overview {
  const overview = value as Overview | null
  return (
    !!overview &&
    typeof overview === 'object' &&
    !!overview.sessions &&
    Array.isArray(overview.sessions.sessions) &&
    !!overview.ports &&
    Array.isArray(overview.ports.listeners)
  )
}

export function useTmuxOverview(iii: ExtensionIii, api: TmuxApi): TmuxOverview {
  const [sessions, setSessions] = useState<SessionsSnapshot | null>(null)
  const [ports, setPorts] = useState<PortsSnapshot | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [live, setLive] = useState(false)
  const mounted = useRef(true)

  const refresh = useCallback(() => {
    void Promise.allSettled([api.listSessions(), api.listPorts()]).then(([sessionsResult, portsResult]) => {
      if (!mounted.current) return
      setLoading(false)
      if (sessionsResult.status === 'fulfilled') {
        setSessions((current) => newerSnapshot(current, sessionsResult.value))
      }
      if (portsResult.status === 'fulfilled') {
        setPorts((current) => newerSnapshot(current, portsResult.value))
      }
      if (sessionsResult.status === 'rejected') setError(errorMessage(sessionsResult.reason))
      else setError(null)
    })
  }, [api])

  useEffect(() => {
    mounted.current = true
    const offHandler = iii.on<Overview>(CHANGED_HANDLER, (payload) => {
      if (!mounted.current || !isOverview(payload)) return
      setSessions((current) => newerSnapshot(current, payload.sessions))
      setPorts((current) => newerSnapshot(current, payload.ports))
      setLoading(false)
      setError(null)
      setLive(true)
    })
    const offTrigger = iii.registerTrigger({
      type: 'tmux::changed',
      function_id: `${CHANGED_HANDLER}::${iii.browserId}`,
      config: {},
    })
    refresh()
    let interrupted = false
    const offConnection = iii.addConnectionStateListener((state) => {
      if (state === 'connected') {
        if (interrupted) {
          interrupted = false
          refresh()
        }
        return
      }
      interrupted = true
    })
    return () => {
      mounted.current = false
      offConnection()
      offTrigger()
      offHandler()
    }
  }, [iii, refresh])

  return { sessions, ports, loading, error, live, refresh }
}
