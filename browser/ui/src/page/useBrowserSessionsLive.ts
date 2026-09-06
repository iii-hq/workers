import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import {
  type BrowserSessionInfo,
  errorMessage,
  listBrowserSessions,
} from '../lib/browser'
import { useBrowserLifecycleEvents } from '../lib/events'

/**
 * Live tab feed for the browser page: `browser::sessions::list`, re-read on
 * session-started / session-stopped / session-updated / navigated. A modest
 * poll runs alongside — faster while the event bindings are unavailable (SDK
 * hiccup, races around worker restart), slower otherwise — so a title a page
 * sets after it loaded reaches the tab strip too. Skipped entirely while the
 * document is hidden so a backgrounded console never hammers the engine.
 */

export const BROWSER_SESSIONS_POLL_MS = 10_000
/** Title catch-up cadence while the live bindings are up. */
export const BROWSER_SESSIONS_TITLE_POLL_MS = 15_000

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
    if (!enabled) return
    const id = window.setInterval(
      () => {
        if (document.hidden) return
        refresh()
      },
      bound ? BROWSER_SESSIONS_TITLE_POLL_MS : BROWSER_SESSIONS_POLL_MS,
    )
    return () => window.clearInterval(id)
  }, [enabled, bound, refresh])

  return { sessions, loading, error, live: bound, refresh }
}
