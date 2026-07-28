import type { BrowserSessionInfo } from '../lib/browser'
import { cn } from '../lib/cn'
import { formatMtime } from '../lib/format'
import { Globe } from '../lib/icons'

/**
 * Left rail: one row per live Chromium session. Selection is page-local
 * state (the injected page owns its own routing), so a click just flips the
 * selected id.
 */

interface SessionRailProps {
  sessions: BrowserSessionInfo[]
  selectedId: string | null
  onSelect: (sessionId: string) => void
}

function hostOf(url: string): string {
  try {
    const parsed = new URL(url)
    return parsed.host || url
  } catch {
    return url
  }
}

export function SessionRail({
  sessions,
  selectedId,
  onSelect,
}: SessionRailProps) {
  return (
    <nav aria-label="browser sessions" className="br-ui-rail">
      {sessions.map((session) => {
        const selected = session.session_id === selectedId
        return (
          <button
            key={session.session_id}
            type="button"
            onClick={() => onSelect(session.session_id)}
            aria-current={selected ? 'true' : undefined}
            title={`${session.session_id} · ${session.url}`}
            className={cn('br-ui-rail-row', selected && 'is-selected')}
          >
            <span className="br-ui-rail-head">
              <Globe
                size={12}
                aria-hidden
                className={cn('br-ui-rail-icon', selected && 'is-selected')}
              />
              <span className="br-ui-rail-title">
                {session.title?.trim() || hostOf(session.url) || 'about:blank'}
              </span>
            </span>
            <span className="br-ui-rail-url">
              <span className="br-ui-truncate">{session.url}</span>
            </span>
            <span className="br-ui-rail-meta">
              <span className="br-ui-num">{session.session_id}</span>
              <span>·</span>
              <span>{session.headless ? 'headless' : 'headful'}</span>
              <span>·</span>
              <span>{formatMtime(Math.floor(session.last_used_ms / 1000))}</span>
            </span>
          </button>
        )
      })}
    </nav>
  )
}
