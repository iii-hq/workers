import type { Host } from '@iii-dev/console-ui'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { useEffect } from 'react'
import {
  BROWSER_LIFECYCLE_TRIGGERS,
  type BrowserSessionInfo,
  listBrowserSessions,
} from '../lib/browser'

/**
 * Live tab feed for the browser page: `browser::sessions::list`, re-read on
 * session-started / session-stopped / session-updated / navigated through
 * the shared `useWorkerLive` (which also polls while the bindings are down).
 * A slow poll runs alongside even while live, because no trigger fires for
 * a title a page sets after it loaded — the tab strip catches it up here.
 * Skipped while the document is hidden.
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

const EMPTY: BrowserSessionInfo[] = []

export function useBrowserSessionsLive(host: Host): BrowserSessionsLive {
  const { data, loading, error, live, refresh } = useWorkerLive({
    iii: host.iii,
    triggers: BROWSER_LIFECYCLE_TRIGGERS,
    fetch: () => listBrowserSessions(host.iii),
    pollMs: BROWSER_SESSIONS_POLL_MS,
    handlerId: 'iii::browser-ui::lifecycle',
  })
  useEffect(() => {
    if (!live) return
    const id = window.setInterval(() => {
      if (!document.hidden) refresh()
    }, BROWSER_SESSIONS_TITLE_POLL_MS)
    return () => window.clearInterval(id)
  }, [live, refresh])
  return { sessions: data ?? EMPTY, loading, error, live, refresh }
}
