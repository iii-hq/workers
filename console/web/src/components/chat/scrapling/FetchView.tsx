import { FilterChip } from '@/components/chat/engine/shared'
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
  fetchEngineLabel,
  fetchRequestSchema,
  fetchResponseSchema,
  formatChars,
  type PageResult,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

const MAX_TARGET_LINES = 5
const HTML_PREVIEW_CHARS = 1500

interface FetchViewProps {
  functionId: string
  input: unknown
  output: unknown
  running?: boolean
}

/** Shared by `scrapling::fetch`, `::stealthy-fetch`, and `::dynamic-fetch` —
 *  same page/bulk response shape; only the request chips differ per engine. */
export function FetchView({
  functionId,
  input,
  output,
  running,
}: FetchViewProps) {
  const req = safeParseRequest(fetchRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="fetching…" variant="default" />
          <OptionChips functionId={functionId} req={req} />
        </MetaRow>
        <TargetLines urls={targetUrls(req)} />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · waiting for page…
        </div>
      </div>
    )
  }

  const result = safeParseResponse(fetchResponseSchema, output)
  if (!result) return null

  if ('results' in result) {
    return (
      <BulkPane functionId={functionId} req={req} results={result.results} />
    )
  }
  return <SinglePane functionId={functionId} req={req} page={result} />
}

/** Compact read-only summary shown while the call sits in the approval gate. */
export function FetchPreview({
  functionId,
  input,
}: {
  functionId: string
  input: unknown
}) {
  const req = safeParseRequest(fetchRequestSchema, input)
  if (!req) return null
  const urls = targetUrls(req)
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={
            urls.length > 1
              ? `permission to fetch ${urls.length} urls`
              : 'permission to fetch'
          }
          variant="warn"
        />
        <OptionChips functionId={functionId} req={req} />
      </MetaRow>
      <TargetLines urls={urls} />
    </div>
  )
}

function targetUrls(req: FetchRequest): string[] {
  if (req.urls?.length) return req.urls
  return req.url ? [req.url] : []
}

function OptionChips({
  functionId,
  req,
}: {
  functionId: string
  req: FetchRequest
}) {
  const method =
    functionId === 'scrapling::fetch'
      ? (req.method ?? 'get').toUpperCase()
      : null
  return (
    <>
      <Chip>{fetchEngineLabel(functionId)}</Chip>
      {method ? <Chip>{method}</Chip> : null}
      {req.impersonate ? (
        <FilterChip label="as" value={req.impersonate} />
      ) : null}
      {req.solve_cloudflare ? (
        <Chip className="text-warn border-warn/40">
          <span className="uppercase tracking-[0.06em]">cloudflare</span>
        </Chip>
      ) : null}
      {req.headless === false ? (
        <Chip className="text-warn border-warn/40">
          <span className="uppercase tracking-[0.06em]">headed</span>
        </Chip>
      ) : null}
      {req.real_chrome ? <Chip>real chrome</Chip> : null}
      {req.cdp_url ? <Chip>cdp</Chip> : null}
      {req.wait_selector ? (
        <FilterChip label="wait" value={req.wait_selector} />
      ) : null}
      {req.network_idle ? <Chip>network idle</Chip> : null}
      {/* proxy values can embed credentials — flag presence, never the value */}
      {req.proxy ? <Chip>proxy</Chip> : null}
      {req.selectors?.length ? (
        <FilterChip label="selectors" value={req.selectors.length} />
      ) : null}
      {req.include_html ? <Chip>html</Chip> : null}
    </>
  )
}

function TargetLines({ urls }: { urls: string[] }) {
  return (
    <>
      {urls.slice(0, MAX_TARGET_LINES).map((url, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; rows never reorder and urls may repeat
        <ActionLine key={`${i}:${url}`} symbol="→" tone="ink">
          <span className="break-all">{url}</span>
        </ActionLine>
      ))}
      {urls.length > MAX_TARGET_LINES ? (
        <div className="px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] text-ink-ghost">
          +{urls.length - MAX_TARGET_LINES} more urls
        </div>
      ) : null}
    </>
  )
}

/* ---------------- single page ---------------- */

function SinglePane({
  functionId,
  req,
  page,
}: {
  functionId: string
  req: FetchRequest
  page: PageResult
}) {
  const status = page.status ?? null
  const contentType = headerValue(page.headers, 'content-type')
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={status != null ? String(status) : 'done'}
          variant={statusToVariant(status)}
        />
        <OptionChips functionId={functionId} req={req} />
        {contentType ? (
          <FilterChip label="type" value={contentType.split(';')[0]} />
        ) : null}
        {page.captured_xhr?.length ? (
          <FilterChip label="xhr" value={page.captured_xhr.length} />
        ) : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{page.url || req.url || ''}</span>
      </ActionLine>
      {page.extracted ? <Extracted extracted={page.extracted} /> : null}
      {page.html != null ? <HtmlSnippet html={page.html} /> : null}
      {!page.extracted && page.html == null ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · page fetched — pass `selectors` or `include_html` for content
        </div>
      ) : null}
    </div>
  )
}

function headerValue(
  headers: Record<string, unknown> | undefined,
  name: string,
): string | null {
  if (!headers) return null
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() === name) return String(value)
  }
  return null
}

function statusToVariant(
  status: number | null,
): 'accent' | 'default' | 'warn' | 'alert' {
  if (status == null) return 'default'
  if (status >= 500) return 'alert'
  if (status >= 400) return 'warn'
  if (status >= 200 && status < 300) return 'accent'
  return 'default'
}

function Extracted({ extracted }: { extracted: Record<string, unknown> }) {
  return (
    <div>
      <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        extracted · {Object.keys(extracted).length}
      </div>
      <JsonHighlight code={JSON.stringify(extracted, null, 2)} wrap />
    </div>
  )
}

function HtmlSnippet({ html }: { html: string }) {
  const truncated = html.length > HTML_PREVIEW_CHARS
  return (
    <div>
      <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        html · {formatChars(html.length)}
        {truncated ? ' · truncated preview' : ''}
      </div>
      <pre className="px-3 py-2 font-mono text-[11.5px] leading-[1.5] text-ink-faint whitespace-pre-wrap break-all m-0">
        <code>
          {html.slice(0, HTML_PREVIEW_CHARS)}
          {truncated ? '…' : ''}
        </code>
      </pre>
    </div>
  )
}

/* ---------------- bulk ---------------- */

function BulkPane({
  functionId,
  req,
  results,
}: {
  functionId: string
  req: FetchRequest
  results: PageResult[]
}) {
  const failed = results.filter((r) => r.error != null).length
  const ok = results.length - failed
  const extractedByUrl: Record<string, unknown> = {}
  for (const r of results) {
    if (r.extracted && r.url) extractedByUrl[r.url] = r.extracted
  }
  const extractedCount = Object.keys(extractedByUrl).length
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={`${ok}/${results.length} ok`}
          variant={failed ? 'warn' : 'accent'}
        />
        <OptionChips functionId={functionId} req={req} />
      </MetaRow>
      <div>
        {results.map((r, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; rows never reorder and urls may repeat
            key={`${i}:${r.url ?? ''}`}
            className="flex items-start gap-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[12px]"
          >
            <span
              className={cn(
                'shrink-0 tabular-nums',
                r.error != null ? 'text-warn' : 'text-ink-faint',
              )}
            >
              {r.error != null ? '✗' : (r.status ?? '·')}
            </span>
            <span className="min-w-0 text-ink break-all">{r.url ?? ''}</span>
            {r.error != null ? (
              <span className="min-w-0 text-warn break-words">{r.error}</span>
            ) : null}
          </div>
        ))}
      </div>
      {extractedCount > 0 ? (
        <div>
          <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
            extracted · {extractedCount} page{extractedCount === 1 ? '' : 's'}
          </div>
          <JsonHighlight code={JSON.stringify(extractedByUrl, null, 2)} wrap />
        </div>
      ) : null}
    </div>
  )
}
