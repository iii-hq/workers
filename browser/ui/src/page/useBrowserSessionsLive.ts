import type { Host } from '@iii-dev/console-ui'
import { useWorkerLive } from '@iii-dev/console-ui/hooks'
import { BROWSER_LIFECYCLE_FUNCTION_IDS, type BrowserSessionInfo, listBrowserSessions } from '../lib/browser'

/**
 * Live tab feed for the browser page: `browser::sessions::list`, re-read on
 * every call of `browser::on-session-started`, `on-session-stopped`,
 * `on-session-updated` and `on-navigated`, observed through the shared
 * `useWorkerLive`. Nothing runs on a timer: a title the page sets after it
 * loaded arrives through `on-session-updated` (the worker watches the page's
 * title), and without live observers the list is re-read when the person
 * comes back to the tab.
 */

export interface BrowserSessionsLive {
  sessions: BrowserSessionInfo[]
  loading: boolean
  error: string | null
  /** True while updates arrive through the observers. */
  live: boolean
  refresh: () => void
}

const EMPTY: BrowserSessionInfo[] = []

export function useBrowserSessionsLive(host: Host): BrowserSessionsLive {
  const { data, loading, error, live, refresh } = useWorkerLive({
    iii: host.iii,
    sources: BROWSER_LIFECYCLE_FUNCTION_IDS,
    fetch: () => listBrowserSessions(host.iii),
    handlerId: 'iii::browser-ui::lifecycle',
  })
  return { sessions: data ?? EMPTY, loading, error, live, refresh }
}
