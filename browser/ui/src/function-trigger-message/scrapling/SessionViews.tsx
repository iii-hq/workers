import type { Host } from '@iii-dev/console-ui'
import { OpenInBrowser } from '../open-in-browser'
import { JsonHighlight } from '@iii-dev/console-ui'
import {
  ActionLine,
  Chip,
  FilterChip,
  MetaRow,
  StatusPill,
} from '../../lib/shared'
import {
  pageResultSchema,
  type SessionSummary,
  safeParseRequest,
  safeParseResponse,
  sessionCloseRequestSchema,
  sessionCloseResponseSchema,
  sessionFetchRequestSchema,
  sessionListRequestSchema,
  sessionListResponseSchema,
  sessionOpenRequestSchema,
  sessionOpenResponseSchema,
} from './parsers'

function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 10)}…` : id
}

function SectionShell({ children }: { children: React.ReactNode }) {
  return <div className="br-ui-scrape-section">{children}</div>
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
        <Chip className="br-ui-scrape-warning">
          <span>cloudflare</span>
        </Chip>
      ) : null}
      {req.real_chrome ? <Chip>real chrome</Chip> : null}
      {req.headless === false ? (
        <Chip className="br-ui-scrape-warning">
          <span>headed</span>
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
  const chips = openChips(input)
  if (chips == null) return null
  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="opening…" variant="default" />
          {chips}
        </MetaRow>
        <div className="br-ui-scrape-running">
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
        {chips}
      </MetaRow>
      <ActionLine symbol="#" tone="accent">
        <span className="br-ui-scrape-break">{res.session_id}</span>
      </ActionLine>
    </SectionShell>
  )
}

export function SessionOpenPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(sessionOpenRequestSchema, input)
  if (!req) return null
  return (
    <div className="br-ui-scrape-section is-preview">
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
} | null {
  const req = safeParseRequest(sessionFetchRequestSchema, input)
  if (!req) return null
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
  host,
}: {
  input: unknown
  output: unknown
  running?: boolean
  host?: Host
}) {
  const header = fetchHeader(input)
  if (!header) return null
  const { url, node } = header

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="fetching…" variant="default" />
          {node}
        </MetaRow>
        {url ? (
          <ActionLine symbol="→" tone="ink">
            <span className="br-ui-scrape-break">{url}</span>
          </ActionLine>
        ) : null}
        <div className="br-ui-scrape-running">
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
        <span className="br-ui-scrape-break">{page.url || url || ''}</span>
        {host && (page.url || url) ? (
          <OpenInBrowser host={host} url={page.url || url || ''} />
        ) : null}
      </ActionLine>
      {page.extracted ? (
        <div>
          <div className="br-ui-scrape-label">
            extracted · {Object.keys(page.extracted).length}
          </div>
          <JsonHighlight code={JSON.stringify(page.extracted, null, 2)} wrap />
        </div>
      ) : null}
      {page.content != null ? (
        <pre className="br-ui-scrape-pre is-separated">
          <code>{page.content.slice(0, 2000)}</code>
        </pre>
      ) : null}
    </SectionShell>
  )
}

export function SessionFetchPreview({ input }: { input: unknown }) {
  const header = fetchHeader(input)
  if (!header) return null
  const { url, node } = header
  return (
    <div className="br-ui-scrape-section is-preview">
      <MetaRow>
        <StatusPill label="permission to fetch" variant="warn" />
        {node}
      </MetaRow>
      {url ? (
        <ActionLine symbol="→" tone="ink">
          <span className="br-ui-scrape-break">{url}</span>
        </ActionLine>
      ) : null}
    </div>
  )
}

/* ---------------- session-close ---------------- */

function closeChip(input: unknown): React.ReactNode | null {
  const req = safeParseRequest(sessionCloseRequestSchema, input)
  if (!req) return null
  return <FilterChip label="session" value={shortId(req.session_id)} />
}

export function SessionCloseView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const chip = closeChip(input)
  if (!chip) return null
  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="closing…" variant="default" />
          {chip}
        </MetaRow>
        <div className="br-ui-scrape-running">· closing session…</div>
      </SectionShell>
    )
  }
  const res = safeParseResponse(sessionCloseResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={res.closed ? 'closed' : 'not found'}
          variant={res.closed ? 'accent' : 'warn'}
        />
        {chip}
      </MetaRow>
    </SectionShell>
  )
}

export function SessionClosePreview({ input }: { input: unknown }) {
  const chip = closeChip(input)
  if (!chip) return null
  return (
    <div className="br-ui-scrape-section is-preview">
      <MetaRow>
        <StatusPill label="permission to close a session" variant="warn" />
        {chip}
      </MetaRow>
    </div>
  )
}

/* ---------------- session-list ---------------- */

export function SessionListView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  if (!safeParseRequest(sessionListRequestSchema, input)) return null
  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="listing…" variant="default" />
        </MetaRow>
        <div className="br-ui-scrape-running">· listing sessions…</div>
      </SectionShell>
    )
  }
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
        <div className="br-ui-scrape-empty">
          · no open sessions
        </div>
      ) : (
        <div className="br-ui-scrape-table-wrap">
          <table className="br-ui-scrape-table">
            <tbody>
              {res.sessions.map((s) => (
                <SessionRow key={s.session_id} s={s} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </SectionShell>
  )
}

export function SessionListPreview({ input }: { input: unknown }) {
  if (!safeParseRequest(sessionListRequestSchema, input)) return null
  return (
    <div className="br-ui-scrape-section is-preview">
      <MetaRow>
        <StatusPill label="permission to list sessions" variant="warn" />
      </MetaRow>
    </div>
  )
}

function SessionRow({ s }: { s: SessionSummary }) {
  return (
    <tr>
      <td className="br-ui-scrape-table-type">{s.type ?? 'http'}</td>
      <td className="br-ui-scrape-table-value">{s.session_id}</td>
      <td className="br-ui-scrape-table-meta">
        {typeof s.idle_s === 'number' ? `idle ${s.idle_s}s` : ''}
      </td>
    </tr>
  )
}
