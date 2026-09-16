import type { Host } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { ArrowRight, Hash } from 'lucide-react'
import { OpenInBrowser } from '../open-in-browser'
import {
  ActionLine,
  Badge,
  Chip,
  EmptyState,
  JsonHighlight,
  MetaRow,
  Table,
  TableBody,
  TableCell,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { cn } from '../../lib/cn'
import { FilterChip } from '../../lib/shared'
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
        <Chip tone="warning">
          <span>cloudflare</span>
        </Chip>
      ) : null}
      {req.real_chrome ? <Chip>real chrome</Chip> : null}
      {req.headless === false ? (
        <Chip tone="warning">
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
          <Badge variant="default">opening…</Badge>
          {chips}
        </MetaRow>
        <div className="br-ui-more">
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
        <Badge variant="accent">session open</Badge>
        {chips}
      </MetaRow>
      <ActionLine icon={<Hash size={16} aria-hidden />} tone="accent">
        <span className="br-ui-break">{res.session_id}</span>
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
        <Badge variant="warn">permission to open a session</Badge>
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
          <Badge variant="default">fetching…</Badge>
          {node}
        </MetaRow>
        {url ? (
          <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
            <span className="br-ui-break">{url}</span>
          </ActionLine>
        ) : null}
        <div className="br-ui-more">
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
        <Badge variant={
            status != null && status >= 200 && status < 300
              ? 'accent'
              : 'default'
          }>{status != null ? String(status) : 'done'}</Badge>
        {node}
      </MetaRow>
      <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
        <span className="br-ui-break">{page.url || url || ''}</span>
        {host && (page.url || url) ? (
          <OpenInBrowser host={host} url={page.url || url || ''} />
        ) : null}
      </ActionLine>
      {page.extracted ? (
        <div>
          <div className={cn('br-ui-scrape-label', uiClasses.eyebrow)}>
            extracted · {Object.keys(page.extracted).length}
          </div>
          <JsonHighlight code={JSON.stringify(page.extracted, null, 2)} wrap />
        </div>
      ) : null}
      {page.content != null ? (
        <pre className="br-ui-text is-separated">
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
        <Badge variant="warn">permission to fetch</Badge>
        {node}
      </MetaRow>
      {url ? (
        <ActionLine icon={<ArrowRight size={16} aria-hidden />} tone="ink">
          <span className="br-ui-break">{url}</span>
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
          <Badge variant="default">closing…</Badge>
          {chip}
        </MetaRow>
        <div className="br-ui-more">· closing session…</div>
      </SectionShell>
    )
  }
  const res = safeParseResponse(sessionCloseResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <Badge variant={res.closed ? 'accent' : 'warn'}>{res.closed ? 'closed' : 'not found'}</Badge>
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
        <Badge variant="warn">permission to close a session</Badge>
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
          <Badge variant="default">listing…</Badge>
        </MetaRow>
        <div className="br-ui-more">· listing sessions…</div>
      </SectionShell>
    )
  }
  const res = safeParseResponse(sessionListResponseSchema, output)
  if (!res) return null
  return (
    <SectionShell>
      <MetaRow>
        <Badge variant={res.sessions.length ? 'accent' : 'default'}>{`${res.sessions.length} open`}</Badge>
      </MetaRow>
      {res.sessions.length === 0 ? (
        <EmptyState
          title="No open sessions"
          description="Open one with browser::session-open."
        />
      ) : (
        <TableViewport className="br-ui-table">
          <Table density="compact">
            <TableBody>
              {res.sessions.map((s) => (
                <SessionRow key={s.session_id} s={s} />
              ))}
            </TableBody>
          </Table>
        </TableViewport>
      )}
    </SectionShell>
  )
}

export function SessionListPreview({ input }: { input: unknown }) {
  if (!safeParseRequest(sessionListRequestSchema, input)) return null
  return (
    <div className="br-ui-scrape-section is-preview">
      <MetaRow>
        <Badge variant="warn">permission to list sessions</Badge>
      </MetaRow>
    </div>
  )
}

function SessionRow({ s }: { s: SessionSummary }) {
  return (
    <TableRow>
      <TableCell className="br-ui-accent br-ui-td-name">{s.type ?? 'http'}</TableCell>
      <TableCell className="br-ui-break">{s.session_id}</TableCell>
      <TableCell className="br-ui-faint br-ui-num br-ui-right br-ui-nowrap">
        {typeof s.idle_s === 'number' ? `idle ${s.idle_s}s` : ''}
      </TableCell>
    </TableRow>
  )
}
