import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  actRequestSchema,
  actResponseSchema,
  parseScreenshotResponse,
  type SessionInfo,
  safeParseRequest,
  safeParseResponse,
  sessionListResponseSchema,
  sessionStartResponseSchema,
  sessionStopResponseSchema,
} from './parsers'

function shortId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 12)}…` : id
}

function SectionShell({ children }: { children: React.ReactNode }) {
  return <div className="border-t border-rule-2 bg-bg">{children}</div>
}

/* ---------------- screenshot / observe ---------------- */

export function ScreenshotView({
  functionId,
  input,
  output,
  running,
}: {
  functionId: string
  input: unknown
  output: unknown
  running?: boolean
}) {
  const observe = functionId === 'computer::observe'

  if (running) {
    const req = safeParseRequest(actRequestSchema, input)
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="capturing…" variant="default" />
          <Chip>{observe ? 'observe' : 'screenshot'}</Chip>
          {req?.session_id ? <Chip>{shortId(req.session_id)}</Chip> : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · waiting for the desktop…
        </div>
      </SectionShell>
    )
  }

  const shot = parseScreenshotResponse(output)
  if (!shot) return null
  const sizeKb = Math.max(
    1,
    Math.round(
      (shot.images.reduce((n, b) => n + (b.data?.length ?? 0), 0) * 3) /
        4 /
        1024,
    ),
  )
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={observe ? 'observe' : 'screenshot'}
          variant="accent"
        />
        <Chip>{shot.mime.replace('image/', '')}</Chip>
        {shot.width != null && shot.height != null ? (
          <Chip className="tabular-nums">
            {shot.width}×{shot.height}
          </Chip>
        ) : null}
        {shot.sessionId ? <Chip>{shortId(shot.sessionId)}</Chip> : null}
        {shot.hasAccessibility ? <Chip>a11y tree</Chip> : null}
        {shot.images.length > 1 ? (
          <Chip>{shot.images.length} tiles</Chip>
        ) : null}
        <Chip>
          <span className="tabular-nums">{sizeKb}</span>
          <span className="ml-0.5">KB</span>
        </Chip>
      </MetaRow>
      <div className="px-3 py-3 space-y-2">
        {shot.images.map((b, i) => (
          <img
            key={i}
            src={`data:${b.mime || shot.mime};base64,${b.data}`}
            alt={shot.caption || 'desktop screenshot'}
            loading="lazy"
            className="max-w-full max-h-[420px] border border-rule-2 bg-paper-2"
          />
        ))}
        {shot.caption ? (
          <div className="font-mono text-[12.5px] text-ink-ghost break-all">
            {shot.caption}
          </div>
        ) : null}
      </div>
    </SectionShell>
  )
}

/* ---------------- act ---------------- */

export function ActView({
  input,
  output,
}: {
  input: unknown
  output: unknown
}) {
  const res = safeParseResponse(actResponseSchema, output)
  if (!res) return null
  const req = safeParseRequest(actRequestSchema, input)
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={res.ok ? 'done' : 'failed'}
          variant={res.ok ? 'accent' : 'alert'}
        />
        {req?.action ? <Chip>{req.action}</Chip> : null}
        {req?.x != null && req?.y != null ? (
          <Chip className="tabular-nums">
            {req.x},{req.y}
          </Chip>
        ) : null}
        {req?.to_x != null && req?.to_y != null ? (
          <Chip className="tabular-nums">
            → {req.to_x},{req.to_y}
          </Chip>
        ) : null}
        {req?.keys?.length ? <Chip>{req.keys.join('+')}</Chip> : null}
      </MetaRow>
      <ActionLine symbol="·" tone="ink">
        {res.detail}
      </ActionLine>
    </SectionShell>
  )
}

/* ---------------- sessions::start ---------------- */

export function SessionStartView({ output }: { output: unknown }) {
  const res = safeParseResponse(sessionStartResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill label="session started" variant="accent" />
        <Chip>{res.os}</Chip>
        <Chip className="tabular-nums">
          {res.screen.width}×{res.screen.height}
        </Chip>
        <Chip>{res.endpoint}</Chip>
      </MetaRow>
      <ActionLine symbol="#" tone="accent">
        <span className="break-all font-mono">{res.session_id}</span>
      </ActionLine>
    </SectionShell>
  )
}

/* ---------------- sessions::list ---------------- */

export function SessionListView({ output }: { output: unknown }) {
  const res = safeParseResponse(sessionListResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={`${res.sessions.length} open`}
          variant={res.sessions.length ? 'accent' : 'default'}
        />
      </MetaRow>
      {res.sessions.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no live sessions
        </div>
      ) : (
        <table className="w-full font-mono text-[11.5px] text-ink">
          <tbody>
            {res.sessions.map((s) => (
              <SessionRow key={s.session_id} s={s} />
            ))}
          </tbody>
        </table>
      )}
    </SectionShell>
  )
}

function SessionRow({ s }: { s: SessionInfo }) {
  return (
    <tr className="border-b border-rule-2 last:border-b-0">
      <td className="px-3 py-1 text-accent whitespace-nowrap">{s.os}</td>
      <td className="px-3 py-1 text-ink break-all">{s.session_id}</td>
      <td className="px-3 py-1 text-ink-faint break-all">{s.endpoint}</td>
      <td className="px-3 py-1 text-ink-faint tabular-nums text-right whitespace-nowrap">
        {s.screen.width}×{s.screen.height}
        {s.screencast_active ? ' · live' : ''}
      </td>
    </tr>
  )
}

/* ---------------- sessions::stop ---------------- */

export function SessionStopView({ output }: { output: unknown }) {
  const res = safeParseResponse(sessionStopResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={res.was_running ? 'stopped' : 'was not running'}
          variant={res.was_running ? 'accent' : 'default'}
        />
      </MetaRow>
    </SectionShell>
  )
}
