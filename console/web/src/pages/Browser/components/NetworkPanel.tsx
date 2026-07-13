import { useEffect, useState } from 'react'
import { type BrowserNetworkEntry, readBrowserNetwork } from '@/lib/browser'
import { cn } from '@/lib/utils'

/**
 * Network history for the selected session: `browser::network::read` with a
 * failed-only toggle. There is no live network trigger, so the panel
 * re-reads on a modest interval while mounted and the tab is visible.
 */

const READ_LIMIT = 200
const NETWORK_POLL_MS = 5_000

function formatTime(timestamp: number): string {
  const date = new Date(timestamp)
  return date.toLocaleTimeString(undefined, { hour12: false })
}

interface NetworkPanelProps {
  sessionId: string
  enabled: boolean
}

export function NetworkPanel({ sessionId, enabled }: NetworkPanelProps) {
  const [failedOnly, setFailedOnly] = useState(false)
  const [entries, setEntries] = useState<BrowserNetworkEntry[]>([])
  const [dropped, setDropped] = useState(0)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    setEntries([])
    const load = async () => {
      try {
        const res = await readBrowserNetwork(sessionId, {
          failedOnly,
          limit: READ_LIMIT,
        })
        if (cancelled || !res) return
        setEntries(res.entries)
        setDropped(res.dropped)
        setError(null)
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
      }
    }
    void load()
    const id = window.setInterval(() => {
      if (document.hidden) return
      void load()
    }, NETWORK_POLL_MS)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [enabled, sessionId, failedOnly])

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
