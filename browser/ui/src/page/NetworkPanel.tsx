import {
  Button,
  EmptyState,
  type Host,
  SearchField,
  SegmentedControl,
  StatusPanel,
  Toolbar,
} from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import {
  BROWSER_NETWORK_EVENT_TRIGGER,
  type BrowserNetworkEntry,
  errorMessage,
  formatTime,
  parseNetworkEvent,
  readBrowserNetwork,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { useBrowserSessionEvent } from '../lib/events'

/**
 * Network feed for the selected session: seeded from `browser::network::read`,
 * then appended through the session-filtered `browser::network-event`
 * binding, the same seed-then-stream shape as the console panel. The
 * failed-only toggle filters both the seed and the live entries.
 */

const SEED_LIMIT = 200
const MAX_ENTRIES = 500
const PATTERN_DEBOUNCE_MS = 300

const NETWORK_FEED_FN = 'iii::browser-ui::network-feed'

interface NetworkPanelProps {
  host: Host
  sessionId: string
  enabled: boolean
}

export function NetworkPanel({ host, sessionId, enabled }: NetworkPanelProps) {
  const [pattern, setPattern] = useState('')
  const [debouncedPattern, setDebouncedPattern] = useState('')
  const [failedOnly, setFailedOnly] = useState(false)
  const [entries, setEntries] = useState<BrowserNetworkEntry[]>([])
  const [dropped, setDropped] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const lastSeqRef = useRef(0)
  const failedOnlyRef = useRef(false)
  failedOnlyRef.current = failedOnly
  const patternRef = useRef('')
  patternRef.current = debouncedPattern

  useEffect(() => {
    const id = window.setTimeout(() => setDebouncedPattern(pattern.trim()), PATTERN_DEBOUNCE_MS)
    return () => window.clearTimeout(id)
  }, [pattern])

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    lastSeqRef.current = 0
    setEntries([])
    void readBrowserNetwork(host.iii, sessionId, {
      pattern: debouncedPattern || undefined,
      failedOnly,
      limit: SEED_LIMIT,
    })
      .then((res) => {
        if (cancelled || !res) return
        setEntries(res.entries)
        setDropped(res.dropped)
        lastSeqRef.current = res.last_seq
        setError(null)
      })
      .catch((err) => {
        if (cancelled) return
        setError(errorMessage(err))
      })
    return () => {
      cancelled = true
    }
  }, [host, enabled, sessionId, failedOnly, debouncedPattern])

  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_NETWORK_EVENT_TRIGGER,
    sessionId,
    fnId: NETWORK_FEED_FN,
    onEvent: (payload) => {
      const evt = parseNetworkEvent(payload)
      if (!evt || evt.session_id !== sessionId) return
      if (evt.entry.seq <= lastSeqRef.current) return
      lastSeqRef.current = evt.entry.seq
      if (failedOnlyRef.current && !evt.entry.failed) return
      if (patternRef.current) {
        try {
          if (!new RegExp(patternRef.current, 'i').test(evt.entry.url)) return
        } catch {
          if (!evt.entry.url.toLowerCase().includes(patternRef.current.toLowerCase())) return
        }
      }
      setEntries((cur) => [...cur.slice(-(MAX_ENTRIES - 1)), evt.entry])
    },
  })

  return (
    <div className="br-ui-panel">
      <Toolbar
        aria-label="network filters"
        end={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setEntries([])
              setDropped(0)
            }}
          >
            Clear
          </Button>
        }
      >
        <span className="br-ui-panel-context">{sessionId}</span>
        <SearchField
          name="network-filter"
          value={pattern}
          onChange={setPattern}
          placeholder="filter requests"
          aria-label="filter network requests"
          className="br-ui-filter-input"
        />
        <SegmentedControl<'all' | 'failed'>
          variant="radio"
          value={failedOnly ? 'failed' : 'all'}
          onChange={(next) => setFailedOnly(next === 'failed')}
          options={[
            { value: 'all', label: 'All', icon: false },
            { value: 'failed', label: 'Failed only', icon: false },
          ]}
          aria-label="request filter"
        />
        <span className="br-ui-panel-count">
          {entries.length} {entries.length === 1 ? 'request' : 'requests'}
        </span>
        {dropped > 0 ? (
          <span className="br-ui-panel-note">{dropped} older requests dropped from the buffer</span>
        ) : null}
      </Toolbar>
      {error ? (
        <div className="br-ui-panel-body">
          <StatusPanel variant="alert" headline="Network read failed" detail={error} />
        </div>
      ) : entries.length === 0 ? (
        <div className="br-ui-panel-body">
          <EmptyState
            title={failedOnly ? 'No failed requests' : 'No requests yet'}
            description={
              failedOnly
                ? 'Every request so far succeeded.'
                : 'Requests the page makes appear here as they happen.'
            }
          />
        </div>
      ) : (
        <div className="br-ui-network-table">
          <div className="br-ui-nhead" aria-hidden>
            <span>Time</span>
            <span>Status</span>
            <span>Method</span>
            <span>Request</span>
            <span>Type</span>
          </div>
          <ul className="br-ui-feed">
            {[...entries].reverse().map((entry) => (
              <li key={entry.seq} className="br-ui-nrow">
                <span className="br-ui-nrow-time">{formatTime(entry.timestamp)}</span>
                <span className={cn('br-ui-nrow-status', entry.failed && 'is-failed')}>
                  {entry.status ?? (entry.failed ? 'err' : '...')}
                </span>
                <span className="br-ui-nrow-method">{entry.method}</span>
                <span className="br-ui-nrow-url" title={entry.url}>
                  {entry.url}
                  {entry.error ? <span className="br-ui-alert"> · {entry.error}</span> : null}
                </span>
                <span className="br-ui-nrow-mime">{entry.mime_type ?? '—'}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
