/**
 * The session's visited pages, newest first: what a browser's history page
 * shows. Seeded from `browser::history::list`, re-seeded on navigation and
 * on the filter. Clicking a row navigates the session there.
 */

import {
  EmptyState,
  type Host,
  List,
  ListItem,
  SearchField,
  StatusPanel,
} from '@iii-dev/console-ui'
import { Globe } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  BROWSER_NAVIGATED_TRIGGER,
  type BrowserHistoryVisit,
  errorMessage,
  formatAgo,
  listBrowserHistory,
  navigateBrowser,
} from '../lib/browser'
import { useBrowserSessionEvent } from '../lib/events'

const HISTORY_FEED_FN = 'iii::browser-ui::history-feed'

interface HistoryPanelProps {
  host: Host
  sessionId: string
  enabled: boolean
}

export function HistoryPanel({ host, sessionId, enabled }: HistoryPanelProps) {
  const [visits, setVisits] = useState<BrowserHistoryVisit[]>([])
  const [query, setQuery] = useState('')
  const [error, setError] = useState<string | null>(null)
  // Only the newest search may touch the list; a slow earlier response
  // must not overwrite what a faster later keystroke already showed.
  const searchRevRef = useRef(0)
  const refresh = useCallback(
    (q: string) => {
      const revision = ++searchRevRef.current
      void listBrowserHistory(host.iii, sessionId, q || undefined)
        .then((list) => {
          if (revision === searchRevRef.current) setVisits(list)
        })
        .catch((e: unknown) => {
          if (revision === searchRevRef.current) setError(errorMessage(e))
        })
    },
    [host, sessionId],
  )
  useEffect(() => {
    setError(null)
    refresh(query)
  }, [refresh, query])
  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_NAVIGATED_TRIGGER,
    sessionId,
    fnId: HISTORY_FEED_FN,
    onEvent: () => refresh(query),
  })
  const go = useCallback(
    (url: string) => {
      void navigateBrowser(host.iii, sessionId, url).catch(() => {})
    },
    [host, sessionId],
  )
  return (
    <div className="br-ui-history">
      <SearchField
        value={query}
        onChange={setQuery}
        placeholder="Search history"
        aria-label="search history"
        className="br-ui-panel-search"
      />
      {error ? (
        <StatusPanel variant="alert" headline="History failed" detail={error} />
      ) : visits.length === 0 ? (
        <EmptyState
          title={query ? 'No pages match' : 'No pages visited yet'}
          description={
            query
              ? 'Try a different search.'
              : 'Pages this tab visits are listed here, newest first.'
          }
        />
      ) : (
        <List className="br-ui-panel-list" aria-label="history">
          {visits.map((v) => (
            <ListItem
              key={`${v.timestamp}-${v.url}`}
              onClick={() => go(v.url)}
              title={v.url}
              leading={<Globe size={16} aria-hidden />}
              label={v.title || v.url}
              description={<span className="br-ui-mono">{v.url}</span>}
              trailing={<span className="br-ui-mono br-ui-num">{formatAgo(v.timestamp)}</span>}
            />
          ))}
        </List>
      )}
    </div>
  )
}
