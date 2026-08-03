import { Badge, Button, StatusDot } from '@iii-dev/console-ui'
import { cn } from '../lib/cn'
import type { ComputerSessionInfo } from '../lib/computer'
import { formatAge, shortEndpoint } from '../lib/format'

/**
 * The left rail: every live session, newest first. Each row carries what
 * decides which desktop you are looking at — where it runs, the guest OS, the
 * coordinate space, and whether the live view is streaming.
 */

interface SessionRailProps {
  sessions: ComputerSessionInfo[]
  selectedId: string | null
  loading: boolean
  busyId: string | null
  onSelect: (sessionId: string) => void
  onStop: (sessionId: string) => void
}

export function SessionRail({
  sessions,
  selectedId,
  loading,
  busyId,
  onSelect,
  onStop,
}: SessionRailProps) {
  if (sessions.length === 0) {
    return (
      <p className="cp-ui-rail-empty">
        {loading ? 'loading sessions...' : 'no sessions yet'}
      </p>
    )
  }

  return (
    <ul className="cp-ui-rail-list">
      {sessions.map((session) => {
        const selected = session.session_id === selectedId
        return (
          <li key={session.session_id}>
            <div className={cn('cp-ui-rail-row', selected && 'is-selected')}>
              <button
                type="button"
                className="cp-ui-rail-pick"
                aria-current={selected}
                onClick={() => onSelect(session.session_id)}
              >
                <span className="cp-ui-rail-head">
                  <StatusDot
                    tone={session.screencast_active ? 'accent' : 'ink'}
                    pulse={session.screencast_active}
                    aria-hidden
                  />
                  <span className="cp-ui-rail-id">{session.session_id}</span>
                  <Badge variant="default" className="cp-ui-pill">
                    {session.os}
                  </Badge>
                </span>
                <span className="cp-ui-rail-meta">
                  {shortEndpoint(session.endpoint)} · {session.screen.width}x
                  {session.screen.height} · {formatAge(session.last_used_ms)}
                </span>
              </button>
              <Button
                variant="ghost"
                size="sm"
                className="cp-ui-rail-stop"
                disabled={busyId === session.session_id}
                onClick={() => onStop(session.session_id)}
              >
                stop
              </Button>
            </div>
          </li>
        )
      })}
    </ul>
  )
}
