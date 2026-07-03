import { FilterChip } from '@/components/chat/engine/shared'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { cn } from '@/lib/utils'
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
  return req.start_urls?.length ?? (req.url ? 1 : 0)
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
        <Chip className="text-warn border-warn/40">
          <span className="uppercase tracking-[0.06em]">off-domain</span>
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
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="crawling…" variant="default" />
          {crawlChips(req)}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · walking the site…
        </div>
      </div>
    )
  }

  const res = safeParseResponse(crawlResponseSchema, output)
  if (!res) return null
  const { stats } = res
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={`${stats.items} items`}
          variant={stats.items ? 'accent' : 'warn'}
        />
        <FilterChip label="crawled" value={stats.crawled} />
        {stats.errors > 0 ? (
          <Chip className="text-warn border-warn/40">
            <span className="uppercase tracking-[0.06em]">
              {stats.errors} err
            </span>
          </Chip>
        ) : null}
        {stats.stopped && stats.stopped !== 'done' ? (
          <Chip className="text-warn border-warn/40">
            <span className="uppercase tracking-[0.06em]">{stats.stopped}</span>
          </Chip>
        ) : null}
        {crawlChips(req)}
      </MetaRow>
      {res.stream?.name ? (
        <ActionLine symbol="≈" tone="accent">
          <span className="font-mono text-[11.5px] text-ink-faint break-all">
            stream {res.stream.name}
            {res.stream.group_id ? ` · ${res.stream.group_id}` : ''}
          </span>
        </ActionLine>
      ) : null}
      {res.items && res.items.length > 0 ? (
        <div>
          <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
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
    <div className="flex items-start gap-2 px-3 py-1.5 border-b border-rule-2 last:border-b-0 font-mono text-[12px]">
      <span
        className={cn(
          'shrink-0 tabular-nums',
          item.error ? 'text-warn' : 'text-ink-faint',
        )}
      >
        {item.error ? '✗' : (item.status ?? '·')}
      </span>
      <div className="min-w-0">
        <div className="text-ink break-all">{item.url}</div>
        {summary ? (
          <div
            className={cn(
              'break-words',
              item.error ? 'text-warn' : 'text-ink-faint',
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
  const seeds = req.start_urls ?? (req.url ? [req.url] : [])
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="permission to crawl" variant="warn" />
        {crawlChips(req)}
      </MetaRow>
      {seeds.slice(0, 5).map((u, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; seed list is fixed
        <ActionLine key={`${i}:${u}`} symbol="→" tone="ink">
          <span className="break-all">{u}</span>
        </ActionLine>
      ))}
    </div>
  )
}
