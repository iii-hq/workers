import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import {
  type BrowserSessionInfo,
  errorMessage,
  listBrowserSessions,
} from '../lib/browser'
import { useBrowserLifecycleEvents } from '../lib/events'

/**
 * Live session feed for the browser page: `browser::sessions::list`, re-read
 * on session-started / session-stopped / navigated. While the event bindings
 * are unavailable (SDK hiccup, races around worker restart) a modest poll
 * keeps the rail honest, skipped entirely while the tab is hidden so a
 * backgrounded console never hammers the engine.
 */

export const BROWSER_SESSIONS_POLL_MS = 10_000

export interface BrowserSessionsLive {
  sessions: BrowserSessionInfo[]
  loading: boolean
  error: string | null
  /** True while updates arrive through the live trigger bindings. */
  live: boolean
  refresh: () => void
}

export function useBrowserSessionsLive(
  host: Host,
  enabled: boolean,
): BrowserSessionsLive {
  const [sessions, setSessions] = useState<BrowserSessionInfo[]>([])
  const [loading, setLoading] = useState(enabled)
  const [error, setError] = useState<string | null>(null)
  const [token, setToken] = useState(0)

  const refresh = useCallback(() => setToken((t) => t + 1), [])

  const { bound } = useBrowserLifecycleEvents({
    host,
    enabled,
    onEvent: refresh,
  })

  // biome-ignore lint/correctness/useExhaustiveDependencies: token is a re-run token (bumped by events, polling, and manual refresh), not read by the effect body
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      return
    }
    setLoading(true)
    let cancelled = false
    void (async () => {
      try {
        const next = await listBrowserSessions(host.iii)
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
    }, BROWSER_SESSIONS_POLL_MS)
    return () => window.clearInterval(id)
  }, [enabled, bound, refresh])

  return { sessions, loading, error, live: bound, refresh }
}
