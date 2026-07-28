import type { Host } from '@iii-dev/console-ui'
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

const NETWORK_FEED_FN = 'iii::browser-ui::network-feed'

interface NetworkPanelProps {
  host: Host
  sessionId: string
  enabled: boolean
}

export function NetworkPanel({ host, sessionId, enabled }: NetworkPanelProps) {
  const [failedOnly, setFailedOnly] = useState(false)
  const [entries, setEntries] = useState<BrowserNetworkEntry[]>([])
  const [dropped, setDropped] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const lastSeqRef = useRef(0)
  const failedOnlyRef = useRef(false)
  failedOnlyRef.current = failedOnly

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    lastSeqRef.current = 0
    setEntries([])
    void readBrowserNetwork(host.iii, sessionId, { failedOnly, limit: SEED_LIMIT })
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
  }, [host, enabled, sessionId, failedOnly])

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
      setEntries((cur) => [...cur.slice(-(MAX_ENTRIES - 1)), evt.entry])
    },
  })

  return (
    <div className="br-ui-panel">
      <div className="br-ui-panel-head">
        <button
          type="button"
          onClick={() => setFailedOnly((v) => !v)}
          aria-pressed={failedOnly}
          className={cn('br-ui-toggle', failedOnly && 'is-on')}
        >
          failed only
        </button>
        {dropped > 0 ? (
          <span className="br-ui-panel-note">
            {dropped} older requests dropped from the buffer
          </span>
        ) : null}
      </div>
      {error ? (
        <p className="br-ui-panel-err">{error}</p>
      ) : entries.length === 0 ? (
        <p className="br-ui-panel-empty">
          {failedOnly ? 'no failed requests' : 'no requests yet'}
        </p>
      ) : (
        <ul className="br-ui-feed">
          {[...entries].reverse().map((entry) => (
            <li key={entry.seq} className="br-ui-nrow">
              <span className="br-ui-nrow-time">
                {formatTime(entry.timestamp)}
              </span>
              <span
                className={cn('br-ui-nrow-status', entry.failed && 'is-failed')}
              >
                {entry.status ?? (entry.failed ? 'err' : '...')}
              </span>
              <span className="br-ui-nrow-method">{entry.method}</span>
              <span className="br-ui-nrow-url" title={entry.url}>
                {entry.url}
                {entry.error ? (
                  <span className="br-ui-alert"> · {entry.error}</span>
                ) : null}
              </span>
              {entry.mime_type ? (
                <span className="br-ui-nrow-mime">{entry.mime_type}</span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
