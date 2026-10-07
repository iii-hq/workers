import type { Host } from '@iii-dev/console-ui'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { BROWSER_LIFECYCLE_TRIGGERS, type BrowserSessionInfo, listBrowserSessions } from '../lib/browser'

/**
 * Live tab feed for the browser page: `browser::sessions::list`, re-read on
 * session-started / session-stopped / session-updated / navigated through
 * the shared `useWorkerLive`. Nothing runs on a timer: a title the page sets
 * after it loaded arrives as `session-updated` (the worker watches the
 * page's title), and without live bindings the list is
 * re-read when the person comes back to the tab.
 */

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
    handlerId: 'iii::browser-ui::lifecycle',
  })
  return { sessions: data ?? EMPTY, loading, error, live, refresh }
}
