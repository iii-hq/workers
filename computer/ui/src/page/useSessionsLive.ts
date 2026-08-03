import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { type ComputerSessionInfo, listSessions } from '../lib/computer'
import { errorMessage } from '../lib/errors'
import { useComputerLifecycleEvents } from '../lib/events'

/**
 * Live session feed for the computer page: `computer::sessions::list`,
 * re-read on session-started / session-stopped. While the event bindings are
 * unavailable (SDK hiccup, races around worker restart) a modest poll keeps
 * the rail honest, skipped while the tab is hidden so a backgrounded console
 * never hammers the engine.
 */

export const SESSIONS_POLL_MS = 10_000

export interface SessionsLive {
  sessions: ComputerSessionInfo[]
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useSessionsLive(host: Host, enabled: boolean): SessionsLive {
  const [sessions, setSessions] = useState<ComputerSessionInfo[]>([])
  const [loading, setLoading] = useState(enabled)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  const { bound } = useComputerLifecycleEvents({
    host,
    enabled,
    onEvent: refresh,
  })

  // `token` is a re-run token bumped by events, polling and manual refresh;
  // the effect body never reads it.
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      return
    }
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const next = await listSessions(host.iii)
        if (cancelled) return
        setSessions(next)
        setError(null)
      } catch (err) {
        if (cancelled) return
        setError(errorMessage(err))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [host, enabled, token])

  useEffect(() => {
    if (!enabled || bound) return
    const id = window.setInterval(() => {
      if (document.hidden) return
      refresh()
    }, SESSIONS_POLL_MS)
    return () => window.clearInterval(id)
  }, [enabled, bound, refresh])

  return { sessions, loading, error, live: bound, refresh }
}
