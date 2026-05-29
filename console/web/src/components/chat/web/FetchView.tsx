import { useState } from 'react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import {
  type FetchRequest,
  type FetchResult,
  fetchErrorVariantLabel,
  fetchRequestSchema,
  fetchResultSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface FetchViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function FetchView({ input, output, running }: FetchViewProps) {
  const req = safeParseRequest(fetchRequestSchema, input)
  if (!req) return null

  const method = (req.method ?? 'GET').toUpperCase()

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="fetching…" variant="default" />
          <Chip>{method}</Chip>
        </MetaRow>
        <ActionLine symbol="→" tone="ink">
          <span className="break-all">{req.url}</span>
        </ActionLine>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · waiting for response…
        </div>
      </div>
    )
  }

  const result = safeParseResponse(fetchResultSchema, output)
  if (!result) return null

  if (!result.ok) {
    return <FetchErrorPane req={req} method={method} error={result} />
  }
  return <FetchSuccessPane req={req} method={method} result={result} />
}

/** Compact preview rendered while a `web::fetch` call sits in the
    approval gate. Read-only summary of method + URL + payload counts. */
export function FetchPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(fetchRequestSchema, input)
  if (!req) return null
  const method = (req.method ?? 'GET').toUpperCase()
  const headerCount = req.headers ? Object.keys(req.headers).length : 0
  const hasBody = req.body != null || req.json !== undefined
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="permission to fetch" variant="warn" />
        <Chip>{method}</Chip>
        {req.response_format ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              format
            </span>
            <span className="ml-1 text-ink">{req.response_format}</span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{req.url}</span>
      </ActionLine>
      <div className="px-3 py-1.5 flex flex-wrap items-center gap-1.5 border-b border-rule-2 bg-paper-2">
        <Chip>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            headers
          </span>
          <span className="ml-1 text-ink tabular-nums">{headerCount}</span>
        </Chip>
        {hasBody ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              body
            </span>
            <span className="ml-1 text-ink">
              {req.json !== undefined ? 'json' : 'text'}
            </span>
          </Chip>
        ) : null}
        {typeof req.timeout_ms === 'number' ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              timeout
            </span>
            <span className="ml-1 text-ink tabular-nums">
              {req.timeout_ms}ms
            </span>
          </Chip>
        ) : null}
        {req.follow_redirects === false ? (
          <Chip>
            <span className="uppercase tracking-[0.06em] text-warn">
              no-redirect
            </span>
          </Chip>
        ) : null}
      </div>
    </div>
  )
}

/* ---------------- success pane ---------------- */

interface FetchSuccessPaneProps {
  req: FetchRequest
  method: string
  result: Extract<FetchResult, { ok: true }>
}

function FetchSuccessPane({ req, method, result }: FetchSuccessPaneProps) {
  const contentType = result.headers['content-type']
  const statusVariant = statusToVariant(result.status)
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={`${result.status} ${result.status_text}`}
          variant={statusVariant}
        />
        <Chip>{method}</Chip>
        {contentType ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              type
            </span>
            <span className="ml-1 text-ink">{contentType.split(';')[0]}</span>
          </Chip>
        ) : null}
        <Chip>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            format
          </span>
          <span className="ml-1 text-ink">{result.response_format}</span>
        </Chip>
        {result.bytes_truncated ? (
          <Chip className="text-warn border-warn/40">
            <span className="uppercase tracking-[0.06em]">truncated</span>
          </Chip>
        ) : null}
        {result.redirect_chain && result.redirect_chain.length > 0 ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              redirects
            </span>
            <span className="ml-1 text-ink tabular-nums">
              {result.redirect_chain.length}
            </span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{req.url}</span>
      </ActionLine>
      {result.parse_error ? (
        <ActionLine symbol="!" tone="warn">
          parse error: {result.parse_error}
        </ActionLine>
      ) : null}
      {result.redirect_chain && result.redirect_chain.length > 0 ? (
        <RedirectChain chain={result.redirect_chain} />
      ) : null}
      <ResponseHeaders headers={result.headers} />
      <Body result={result} />
    </div>
  )
}

function RedirectChain({ chain }: { chain: string[] }) {
  return (
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 flex flex-col gap-0.5">
      <div className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        redirect chain
      </div>
      {chain.map((url) => (
        <div
          key={url}
          className="font-mono text-[11.5px] text-ink-faint break-all"
        >
          → {url}
        </div>
      ))}
    </div>
  )
}

function ResponseHeaders({ headers }: { headers: Record<string, string> }) {
  const [open, setOpen] = useState(false)
  const entries = Object.entries(headers)
  if (entries.length === 0) return null
  return (
    <div className="border-b border-rule-2 bg-paper-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full px-3 py-1.5 flex items-center justify-between gap-2 text-left hover:bg-paper-3 cursor-pointer"
        aria-expanded={open}
      >
        <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
          headers · {entries.length}
        </span>
        <span
          aria-hidden
          className={cn(
            'text-ink-ghost transition-transform duration-150 inline-block text-[10px]',
            open && 'rotate-90',
          )}
        >
          ▸
        </span>
      </button>
      {open ? (
        <table className="w-full font-mono text-[11.5px] text-ink border-t border-rule-2">
          <tbody>
            {entries.map(([key, value]) => (
              <tr key={key} className="border-b border-rule-2 last:border-b-0">
                <td className="px-3 py-1 text-ink-faint align-top w-[30%]">
                  {key}
                </td>
                <td className="px-3 py-1 text-ink break-all">{value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
    </div>
  )
}

function Body({ result }: { result: Extract<FetchResult, { ok: true }> }) {
  if (result.response_format === 'base64') {
    return (
      <div className="px-3 py-2 border-b border-rule-2">
        <div className="flex items-center gap-2 mb-1.5">
          <Chip>
            <span className="uppercase tracking-[0.06em]">base64</span>
          </Chip>
          <span className="font-mono text-[11px] text-ink-faint tabular-nums">
            {result.body.length} chars
          </span>
        </div>
        <pre className="font-mono text-[11.5px] leading-[1.5] text-ink-faint whitespace-pre-wrap break-all m-0">
          <code>{truncateString(result.body, 600)}</code>
        </pre>
      </div>
    )
  }

  const jsonString = jsonStringFromResult(result)
  if (jsonString != null) {
    return <JsonHighlight code={jsonString} wrap />
  }

  if (result.body.length === 0) {
    return (
      <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
        · empty body
      </div>
    )
  }

  return (
    <pre className="font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words m-0 px-3 py-2 bg-bg">
      <code>{result.body}</code>
    </pre>
  )
}

function jsonStringFromResult(
  result: Extract<FetchResult, { ok: true }>,
): string | null {
  if (result.response_format === 'json' && result.json !== undefined) {
    try {
      return JSON.stringify(result.json, null, 2)
    } catch {
      // fall through to body parse below
    }
  }
  const contentType = result.headers['content-type']
  if (contentType?.toLowerCase().includes('json') && result.body.length > 0) {
    try {
      return JSON.stringify(JSON.parse(result.body), null, 2)
    } catch {
      return null
    }
  }
  return null
}

function truncateString(value: string, max: number): string {
  if (value.length <= max) return value
  return `${value.slice(0, max)}…`
}

function statusToVariant(
  status: number,
): 'accent' | 'default' | 'warn' | 'alert' {
  if (status >= 500) return 'alert'
  if (status >= 400) return 'warn'
  if (status >= 300) return 'default'
  if (status >= 200) return 'accent'
  return 'default'
}

/* ---------------- error pane ---------------- */

interface FetchErrorPaneProps {
  req: FetchRequest
  method: string
  error: Extract<FetchResult, { ok: false }>
}

function FetchErrorPane({ req, method, error }: FetchErrorPaneProps) {
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={fetchErrorVariantLabel(error.error)}
          variant="warn"
        />
        <Chip>{method}</Chip>
        {typeof error.status === 'number' ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              status
            </span>
            <span className="ml-1 text-ink tabular-nums">{error.status}</span>
          </Chip>
        ) : null}
        <Chip>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            code
          </span>
          <span className="ml-1 text-ink">{error.error}</span>
        </Chip>
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{req.url}</span>
      </ActionLine>
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-warn ml-0">
        <pre className="font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words m-0">
          <code>{error.message}</code>
        </pre>
      </div>
    </div>
  )
}
