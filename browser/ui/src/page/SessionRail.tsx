/**
 * The navigation rail's session list: one row per live Chromium session, in
 * the order the worker lists them. Each row carries what identifies the
 * session — page title (or host), url, session id, headless/headful, last
 * use. Selection is page-local state (the injected page owns its own
 * routing), so a click just flips the selected id; the look follows the
 * redesign's nav-row pattern: wash + accent bar + stronger title, never
 * color alone.
 */

import type { BrowserSessionInfo } from '../lib/browser'
import { cn } from '../lib/cn'
import { formatMtime } from '../lib/format'
import { Globe } from '../lib/icons'

interface SessionRailProps {
  sessions: BrowserSessionInfo[]
  selectedId: string | null
  loading: boolean
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
  loading,
  onSelect,
}: SessionRailProps) {
  if (sessions.length === 0) {
    if (loading) {
      return (
        <div className="br-ui-rail-skel" aria-hidden>
          {[0, 1, 2].map((i) => (
            <div key={i} className="br-ui-skel-row">
              <span className={`bar w${[60, 75, 40][i]}`} />
              <span className={`bar w${[90, 85, 70][i]}`} />
            </div>
          ))}
        </div>
      )
    }
    return (
      <div className="br-ui-rail-empty">
        <p>No sessions yet.</p>
        <p className="dim">
          Sessions started by agents appear in this list live; new session
          starts one now.
        </p>
      </div>
    )
  }

  return (
    <ul className="br-ui-nav-list">
      {sessions.map((session) => {
        const selected = session.session_id === selectedId
        return (
          <li key={session.session_id}>
            <button
              type="button"
              onClick={() => onSelect(session.session_id)}
              aria-current={selected ? 'true' : undefined}
              title={`${session.session_id} · ${session.url}`}
              className={cn('br-ui-rail-row', selected && 'active')}
            >
              <span className="br-ui-rail-head">
                <Globe size={12} aria-hidden className="br-ui-rail-icon" />
                <span className="br-ui-rail-title">
                  {session.title?.trim() || hostOf(session.url) || 'about:blank'}
                </span>
              </span>
              <span className="br-ui-rail-url">{session.url}</span>
              <span className="br-ui-rail-meta">
                <span className="br-ui-num">{session.session_id}</span>
                <span>·</span>
                <span>{session.headless ? 'headless' : 'headful'}</span>
                <span>·</span>
                <span>{formatMtime(Math.floor(session.last_used_ms / 1000))}</span>
              </span>
            </button>
          </li>
        )
      })}
    </ul>
  )
}
