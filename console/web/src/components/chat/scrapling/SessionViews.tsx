import { FilterChip } from '@/components/chat/engine/shared'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  pageResultSchema,
  type SessionSummary,
  safeParseRequest,
  safeParseResponse,
  sessionCloseResponseSchema,
  sessionFetchRequestSchema,
  sessionListResponseSchema,
  sessionOpenRequestSchema,
  sessionOpenResponseSchema,
} from './parsers'

function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 10)}…` : id
}

function SectionShell({ children }: { children: React.ReactNode }) {
  return <div className="border-t border-rule-2 bg-bg">{children}</div>
}

/* ---------------- session-open ---------------- */

function openChips(input: unknown): React.ReactNode {
  const req = safeParseRequest(sessionOpenRequestSchema, input)
  if (!req) return null
  return (
    <>
      <Chip>{req.type ?? 'http'}</Chip>
      {req.impersonate ? (
        <FilterChip label="as" value={req.impersonate} />
      ) : null}
      {req.solve_cloudflare ? (
        <Chip className="text-warn border-warn/40">
          <span className="uppercase tracking-[0.06em]">cloudflare</span>
        </Chip>
      ) : null}
      {req.real_chrome ? <Chip>real chrome</Chip> : null}
      {req.headless === false ? (
        <Chip className="text-warn border-warn/40">
          <span className="uppercase tracking-[0.06em]">headed</span>
        </Chip>
      ) : null}
      {req.proxy ? <Chip>proxy</Chip> : null}
    </>
  )
}

export function SessionOpenView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="opening…" variant="default" />
          {openChips(input)}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · starting session…
        </div>
      </SectionShell>
    )
  }
  const res = safeParseResponse(sessionOpenResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill label="session open" variant="accent" />
        {openChips(input)}
      </MetaRow>
      <ActionLine symbol="#" tone="accent">
        <span className="break-all font-mono">{res.session_id}</span>
      </ActionLine>
    </SectionShell>
  )
}

export function SessionOpenPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(sessionOpenRequestSchema, input)
  if (!req) return null
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="permission to open a session" variant="warn" />
        {openChips(input)}
      </MetaRow>
    </div>
  )
}

/* ---------------- session-fetch ---------------- */

function fetchHeader(input: unknown): {
  sessionId?: string
  url?: string
  node: React.ReactNode
} {
  const req = safeParseRequest(sessionFetchRequestSchema, input)
  return {
    sessionId: req?.session_id,
    url: req?.url,
    node: (
      <>
        {req?.session_id ? (
          <FilterChip label="session" value={shortId(req.session_id)} />
        ) : null}
        {req?.method ? <Chip>{req.method.toUpperCase()}</Chip> : null}
        {req?.selectors?.length ? (
          <FilterChip label="selectors" value={req.selectors.length} />
        ) : null}
        {req?.format ? <FilterChip label="as" value={req.format} /> : null}
      </>
    ),
  }
}

export function SessionFetchView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const { url, node } = fetchHeader(input)

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="fetching…" variant="default" />
          {node}
        </MetaRow>
        {url ? (
          <ActionLine symbol="→" tone="ink">
            <span className="break-all">{url}</span>
          </ActionLine>
        ) : null}
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · waiting for page…
        </div>
      </SectionShell>
    )
  }

  const page = safeParseResponse(pageResultSchema, output)
  if (!page) return null
  const status = page.status ?? null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={status != null ? String(status) : 'done'}
          variant={
            status != null && status >= 200 && status < 300
              ? 'accent'
              : 'default'
          }
        />
        {node}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{page.url || url || ''}</span>
      </ActionLine>
      {page.extracted ? (
        <div>
          <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
            extracted · {Object.keys(page.extracted).length}
          </div>
          <JsonHighlight code={JSON.stringify(page.extracted, null, 2)} wrap />
        </div>
      ) : null}
      {page.content != null ? (
        <pre className="px-3 py-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-words m-0 border-t border-rule-2">
          <code>{page.content.slice(0, 2000)}</code>
        </pre>
      ) : null}
    </SectionShell>
  )
}

export function SessionFetchPreview({ input }: { input: unknown }) {
  const { url, node } = fetchHeader(input)
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="permission to fetch" variant="warn" />
        {node}
      </MetaRow>
      {url ? (
        <ActionLine symbol="→" tone="ink">
          <span className="break-all">{url}</span>
        </ActionLine>
      ) : null}
    </div>
  )
}

/* ---------------- session-close ---------------- */

export function SessionCloseView({ output }: { output: unknown }) {
  const res = safeParseResponse(sessionCloseResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={res.closed ? 'closed' : 'not found'}
          variant={res.closed ? 'accent' : 'warn'}
        />
      </MetaRow>
    </SectionShell>
  )
}

/* ---------------- session-list ---------------- */

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
          · no open sessions
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

function SessionRow({ s }: { s: SessionSummary }) {
  return (
    <tr className="border-b border-rule-2 last:border-b-0">
      <td className="px-3 py-1 text-accent w-[22%]">{s.type ?? 'http'}</td>
      <td className="px-3 py-1 text-ink break-all">{s.session_id}</td>
      <td className="px-3 py-1 text-ink-faint tabular-nums text-right whitespace-nowrap">
        {typeof s.idle_s === 'number' ? `idle ${s.idle_s}s` : ''}
      </td>
    </tr>
  )
}
