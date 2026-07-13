import { useEffect, useRef, useState } from 'react'
import { useBrowserSessionEvent } from '@/hooks/use-browser-events'
import {
  BROWSER_NETWORK_EVENT_TRIGGER,
  type BrowserNetworkEntry,
  errorMessage,
  formatTime,
  parseNetworkEvent,
  readBrowserNetwork,
} from '@/lib/browser'
import { cn } from '@/lib/utils'

/**
 * Network feed for the selected session: seeded from `browser::network::read`,
 * then appended through the session-filtered `browser::network-event`
 * binding, the same seed-then-stream shape as the console panel. The
 * failed-only toggle filters both the seed and the live entries.
 */

const SEED_LIMIT = 200
const MAX_ENTRIES = 500

const NETWORK_FEED_FN = 'console::browser-network-feed'

interface NetworkPanelProps {
  sessionId: string
  enabled: boolean
}

export function NetworkPanel({ sessionId, enabled }: NetworkPanelProps) {
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
    void readBrowserNetwork(sessionId, { failedOnly, limit: SEED_LIMIT })
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
  }, [enabled, sessionId, failedOnly])

  useBrowserSessionEvent({
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
    <div className="flex flex-col h-full min-h-0">
      <div className="shrink-0 flex items-center gap-2 px-3 py-2 border-b border-rule-2">
        <button
          type="button"
          onClick={() => setFailedOnly((v) => !v)}
          aria-pressed={failedOnly}
          className={cn(
            'font-mono text-[11px] lowercase h-7 px-2.5 border transition-colors',
            failedOnly
              ? 'bg-ink text-bg border-ink'
              : 'bg-transparent text-ink-faint border-rule hover:text-ink hover:border-ink',
          )}
        >
          failed only
        </button>
        {dropped > 0 ? (
          <span className="font-mono text-[11px] lowercase text-ink-ghost">
            {dropped} older requests dropped from the buffer
          </span>
        ) : null}
      </div>
      {error ? (
        <p className="px-3 py-2 font-mono text-[12px] text-alert">{error}</p>
      ) : entries.length === 0 ? (
        <p className="px-3 py-2 font-mono text-[12px] lowercase text-ink-ghost">
          {failedOnly ? 'no failed requests' : 'no requests yet'}
        </p>
      ) : (
        <ul className="flex-1 min-h-0 overflow-y-auto flex flex-col-reverse">
          {[...entries].reverse().map((entry) => (
            <li
              key={entry.seq}
              className="flex items-start gap-2 px-3 py-1 border-t border-rule-2 font-mono text-[12px] leading-[1.55]"
            >
              <span className="shrink-0 tabular-nums text-ink-ghost">
                {formatTime(entry.timestamp)}
              </span>
              <span
                className={cn(
                  'shrink-0 w-[42px] tabular-nums',
                  entry.failed ? 'text-alert' : 'text-ink-faint',
                )}
              >
                {entry.status ?? (entry.failed ? 'err' : '...')}
              </span>
              <span className="shrink-0 w-[56px] text-ink-faint">
                {entry.method}
              </span>
              <span
                className="min-w-0 flex-1 truncate text-ink"
                title={entry.url}
              >
                {entry.url}
                {entry.error ? (
                  <span className="text-alert"> · {entry.error}</span>
                ) : null}
              </span>
              {entry.mime_type ? (
                <span className="shrink-0 text-ink-ghost">
                  {entry.mime_type}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
