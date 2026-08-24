/**
 * The session's visited pages, newest first: what a browser's history page
 * shows. Seeded from `browser::history::list`, re-seeded on navigation and
 * on the filter. Clicking a row navigates the session there.
 */

import { type Host, Input } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import {
  BROWSER_NAVIGATED_TRIGGER,
  type BrowserHistoryVisit,
  errorMessage,
  listBrowserHistory,
  navigateBrowser,
} from '../lib/browser'
import { useBrowserSessionEvent } from '../lib/events'
import { formatMtime } from '../lib/format'
import { Globe, Search } from '../lib/icons'

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
  const refresh = useCallback(
    (q: string) => {
      void listBrowserHistory(host.iii, sessionId, q || undefined)
        .then(setVisits)
        .catch((e: unknown) => setError(errorMessage(e)))
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
      <div className="br-ui-history-search">
        <Search size={16} aria-hidden className="br-ui-history-search-icon" />
        <Input
          value={query}
          onChange={setQuery}
          placeholder="Search history"
          aria-label="search history"
          preserveCase
          className="br-ui-history-search-input"
        />
      </div>
      {error ? (
        <p className="br-ui-history-empty">history failed: {error}</p>
      ) : visits.length === 0 ? (
        <p className="br-ui-history-empty">
          {query ? 'No pages match.' : 'No pages visited yet.'}
        </p>
      ) : (
        <ul className="br-ui-history-list" aria-label="history">
          {visits.map((v) => (
            <li key={`${v.timestamp}-${v.url}`}>
              <button
                type="button"
                className="br-ui-history-row"
                onClick={() => go(v.url)}
                title={v.url}
              >
                <Globe size={16} aria-hidden className="br-ui-history-icon" />
                <span className="br-ui-history-text">
                  <span className="br-ui-history-title">
                    {v.title || v.url}
                  </span>
                  <span className="br-ui-history-url">{v.url}</span>
                </span>
                <span className="br-ui-history-time">
                  {formatMtime(Math.floor(v.timestamp / 1000))}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
