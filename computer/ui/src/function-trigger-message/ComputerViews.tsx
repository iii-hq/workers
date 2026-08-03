import { Badge } from '@iii-dev/console-ui'
import { z } from 'zod'
import {
  actResultSchema,
  type ComputerSessionInfo,
  decodeComputerResult,
  displayInfoSchema,
  parseCapture,
  sessionInfoSchema,
  sessionStartSchema,
  sessionStopSchema,
} from '../lib/computer'
import { shortEndpoint } from '../lib/format'

/**
 * Per-function bodies for `computer::*` chat cards. Each view parses the
 * worker's own result shape and returns `null` when it does not match, so the
 * caller falls back to the decoded JSON rather than rendering a half-card.
 */

export function CaptureView({ output }: { output: unknown }) {
  const capture = parseCapture(output)
  if (!capture?.dataUrl) return null
  return (
    <figure className="cp-ui-shot">
      <img
        src={capture.dataUrl}
        alt={`desktop of session ${capture.sessionId}`}
        className="cp-ui-shot-img"
      />
      {capture.width > 0 ? (
        <figcaption className="cp-ui-shot-cap">
          {capture.width}x{capture.height}
        </figcaption>
      ) : null}
    </figure>
  )
}

export function SessionStartView({ output }: { output: unknown }) {
  const parsed = sessionStartSchema.safeParse(decodeComputerResult(output))
  if (!parsed.success) return null
  const session = parsed.data
  return (
    <div className="cp-ui-kv">
      <Kv label="session" value={session.session_id} />
      <Kv label="driving" value={shortEndpoint(session.endpoint)} />
      <Kv label="os" value={session.os} />
      <Kv
        label="screen"
        value={`${session.screen.width}x${session.screen.height}`}
      />
    </div>
  )
}

export function SessionStopView({ output }: { output: unknown }) {
  const parsed = sessionStopSchema.safeParse(decodeComputerResult(output))
  if (!parsed.success) return null
  return (
    <p className="cp-ui-line">
      {parsed.data.was_running ? 'session stopped' : 'already stopped'}
    </p>
  )
}

export function SessionListView({ output }: { output: unknown }) {
  const parsed = z
    .object({ sessions: z.array(sessionInfoSchema) })
    .safeParse(decodeComputerResult(output))
  if (!parsed.success) return null
  const sessions: ComputerSessionInfo[] = parsed.data.sessions
  if (sessions.length === 0) {
    return <p className="cp-ui-line">no live sessions</p>
  }
  return (
    <ul className="cp-ui-list">
      {sessions.map((session) => (
        <li key={session.session_id} className="cp-ui-list-row">
          <span className="cp-ui-list-id">{session.session_id}</span>
          <span className="cp-ui-list-meta">
            {shortEndpoint(session.endpoint)} · {session.os} ·{' '}
            {session.screen.width}x{session.screen.height}
          </span>
          {session.screencast_active ? (
            <Badge variant="accent" className="cp-ui-pill">
              streaming
            </Badge>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

export function ActView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const parsed = actResultSchema.safeParse(decodeComputerResult(output))
  if (!parsed.success) return null
  const action =
    input && typeof input === 'object'
      ? (input as Record<string, unknown>).action
      : undefined
  return (
    <p className="cp-ui-line">
      {typeof action === 'string' ? (
        <Badge variant="default" className="cp-ui-pill">
          {action}
        </Badge>
      ) : null}
      <span className="cp-ui-detail">{parsed.data.detail}</span>
    </p>
  )
}

export function DisplaysView({ output }: { output: unknown }) {
  const parsed = z
    .object({ displays: z.array(displayInfoSchema) })
    .safeParse(decodeComputerResult(output))
  if (!parsed.success) return null
  if (parsed.data.displays.length === 0) {
    return <p className="cp-ui-line">no local displays (not a desktop host)</p>
  }
  return (
    <ul className="cp-ui-list">
      {parsed.data.displays.map((display) => (
        <li key={display.index} className="cp-ui-list-row">
          <span className="cp-ui-list-id">{display.index}</span>
          <span className="cp-ui-list-meta">
            {display.name || 'display'} · {display.width}x{display.height}
          </span>
          {display.primary ? (
            <Badge variant="default" className="cp-ui-pill">
              primary
            </Badge>
          ) : null}
        </li>
      ))}
    </ul>
  )
}

function Kv({ label, value }: { label: string; value: string }) {
  return (
    <div className="cp-ui-kv-row">
      <span className="cp-ui-kv-key">{label}</span>
      <span className="cp-ui-kv-val">{value}</span>
    </div>
  )
}
