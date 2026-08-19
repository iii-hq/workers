import { cn } from '../../lib/cn'
import {
  ActionLine,
  Chip,
  FilterChip,
  MetaRow,
  StatusPill,
} from '../../lib/shared'
import {
  type CrawlItem,
  type CrawlRequest,
  crawlRequestSchema,
  crawlResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

const MAX_ITEM_ROWS = 20

function startCount(req: CrawlRequest): number {
  // ?? only guards null/undefined, so an empty start_urls array must fall
  // through to the single `url` seed (the worker does the same).
  if (req.start_urls && req.start_urls.length > 0) return req.start_urls.length
  return req.url ? 1 : 0
}

function crawlChips(req: CrawlRequest) {
  return (
    <>
      <Chip>{req.fetcher ?? 'http'}</Chip>
      <FilterChip label="seeds" value={startCount(req)} />
      {typeof req.max_pages === 'number' ? (
        <FilterChip label="max" value={`${req.max_pages}p`} />
      ) : null}
      {typeof req.max_depth === 'number' ? (
        <FilterChip label="depth" value={req.max_depth} />
      ) : null}
      {req.selectors?.length ? (
        <FilterChip label="selectors" value={req.selectors.length} />
      ) : null}
      {req.allowed_domains?.length ? (
        <FilterChip label="domains" value={req.allowed_domains.join(', ')} />
      ) : req.same_domain === false ? (
        <Chip className="br-ui-scrape-warning">
          <span>off-domain</span>
        </Chip>
      ) : null}
    </>
  )
}

export function CrawlView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const req = safeParseRequest(crawlRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <div className="br-ui-scrape-section">
        <MetaRow>
          <StatusPill label="crawling…" variant="default" />
          {crawlChips(req)}
        </MetaRow>
        <div className="br-ui-scrape-running">
          · walking the site…
        </div>
      </div>
    )
  }

  const res = safeParseResponse(crawlResponseSchema, output)
  if (!res) return null
  const { stats } = res
  return (
    <div className="br-ui-scrape-section">
      <MetaRow>
        <StatusPill
          label={`${stats.items} items`}
          variant={stats.items ? 'accent' : 'warn'}
        />
        <FilterChip label="crawled" value={stats.crawled} />
        {stats.errors > 0 ? (
          <Chip className="br-ui-scrape-warning">
            <span>{stats.errors} err</span>
          </Chip>
        ) : null}
        {stats.stopped && stats.stopped !== 'done' ? (
          <Chip className="br-ui-scrape-warning">
            <span>{stats.stopped}</span>
          </Chip>
        ) : null}
        {crawlChips(req)}
      </MetaRow>
      {res.stream?.name ? (
        <ActionLine symbol="≈" tone="accent">
          <span className="br-ui-scrape-detail">
            stream {res.stream.name}
            {res.stream.group_id ? ` · ${res.stream.group_id}` : ''}
          </span>
        </ActionLine>
      ) : null}
      {res.items && res.items.length > 0 ? (
        <div>
          <div className="br-ui-scrape-label">
            sample · {res.items.length}
          </div>
          {res.items.slice(0, MAX_ITEM_ROWS).map((item, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; rows never reorder and urls may repeat
            <CrawlRow key={`${i}:${item.url ?? ''}`} item={item} />
          ))}
        </div>
      ) : null}
    </div>
  )
}

function CrawlRow({ item }: { item: CrawlItem }) {
  const summary = item.error
    ? item.error
    : item.extracted
      ? Object.entries(item.extracted)
          .map(([k, v]) => `${k}=${valuePreview(v)}`)
          .join('  ')
      : ''
  return (
    <div className="br-ui-scrape-result-row">
      <span
        className={cn(
          'br-ui-scrape-row-status',
          item.error && 'is-warn',
        )}
      >
        {item.error ? '✗' : (item.status ?? '·')}
      </span>
      <div className="br-ui-scrape-row-main">
        <div className="br-ui-scrape-break">{item.url}</div>
        {summary ? (
          <div
            className={cn(
              'br-ui-scrape-summary',
              item.error && 'is-warn',
            )}
          >
            {summary}
          </div>
        ) : null}
      </div>
    </div>
  )
}

function valuePreview(v: unknown): string {
  if (v == null) return '∅'
  if (Array.isArray(v)) return `[${v.length}]`
  return String(v).slice(0, 60)
}

export function CrawlPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(crawlRequestSchema, input)
  if (!req) return null
  const seeds =
    req.start_urls && req.start_urls.length > 0
      ? req.start_urls
      : req.url
        ? [req.url]
        : []
  return (
    <div className="br-ui-scrape-section is-preview">
      <MetaRow>
        <StatusPill label="permission to crawl" variant="warn" />
        {crawlChips(req)}
      </MetaRow>
      {seeds.slice(0, 5).map((u, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; seed list is fixed
        <ActionLine key={`${i}:${u}`} symbol="→" tone="ink">
          <span className="br-ui-scrape-break">{u}</span>
        </ActionLine>
      ))}
    </div>
  )
}
