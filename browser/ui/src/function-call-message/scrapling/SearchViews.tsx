import {
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
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { cn } from '../../lib/cn'
import { FilterChip } from '../../lib/shared'
import {
  describeRequestSchema,
  describeResponseSchema,
  elementsResponseSchema,
  findByRegexRequestSchema,
  findByTextRequestSchema,
  findRequestSchema,
  formatChars,
  type ScrapedElement,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

const MAX_ROWS = 30

function SectionShell({ children }: { children: React.ReactNode }) {
  return <div className="br-ui-scrape-section">{children}</div>
}

function RunningNote({ label }: { label: string }) {
  return (
    <div className="br-ui-more">
      · {label}
    </div>
  )
}

/* ---------------- find / find-by-text / find-by-regex ---------------- */

function searchChips(functionId: string, input: unknown): React.ReactNode {
  if (functionId === 'browser::find') {
    const req = safeParseRequest(findRequestSchema, input)
    if (!req) return null
    const tag = Array.isArray(req.tag) ? req.tag.join(', ') : req.tag
    return (
      <>
        {tag ? <FilterChip label="tag" value={tag} /> : null}
        {req.attrs
          ? Object.entries(req.attrs).map(([k, v]) => (
              <FilterChip key={k} label={k} value={String(v)} />
            ))
          : null}
        {req.text_regex ? (
          <FilterChip label="text~" value={req.text_regex} />
        ) : null}
        {req.html != null ? (
          <FilterChip label="html" value={formatChars(req.html.length)} />
        ) : null}
      </>
    )
  }
  if (functionId === 'browser::find-by-text') {
    const req = safeParseRequest(findByTextRequestSchema, input)
    if (!req) return null
    return (
      <>
        {req.text ? <FilterChip label="text" value={req.text} /> : null}
        {req.partial ? <Chip>partial</Chip> : null}
        {req.case_sensitive ? <Chip>case</Chip> : null}
      </>
    )
  }
  const req = safeParseRequest(findByRegexRequestSchema, input)
  if (!req) return null
  return (
    <>
      {req.pattern ? <FilterChip label="pattern" value={req.pattern} /> : null}
      {req.case_sensitive ? <Chip>case</Chip> : null}
    </>
  )
}

export function ElementsView({
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
  const chips = searchChips(functionId, input)
  if (chips == null) return null

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <Badge variant="default">searching…</Badge>
          {chips}
        </MetaRow>
        <RunningNote label="scanning DOM…" />
      </SectionShell>
    )
  }

  const res = safeParseResponse(elementsResponseSchema, output)
  if (!res) return null
  const shown = res.items.slice(0, MAX_ROWS)
  return (
    <SectionShell>
      <MetaRow>
        <Badge variant={res.count ? 'accent' : 'warn'}>{
            res.count === 0
              ? 'no match'
              : `${res.count} element${res.count === 1 ? '' : 's'}`
          }</Badge>
        {chips}
      </MetaRow>
      {res.count === 0 ? (
        <EmptyState
          title="No elements matched"
          description="Try a broader tag, attribute, or text filter."
        />
      ) : (
        <>
          {shown.map((el, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; rows never reorder and selectors may repeat
            <ElementRow key={`${i}:${el.css ?? ''}`} el={el} />
          ))}
          {res.items.length > MAX_ROWS ? (
            <div className="br-ui-more">
              +{res.items.length - MAX_ROWS} more
            </div>
          ) : null}
        </>
      )}
    </SectionShell>
  )
}

function ElementRow({ el }: { el: ScrapedElement }) {
  return (
    <div className="br-ui-row br-ui-col">
      <div className="br-ui-scrape-element-main">
        {el.tag ? (
          <span className="br-ui-accent">{el.tag}</span>
        ) : null}
        <span className="br-ui-break">{el.text || '—'}</span>
      </div>
      {el.css ? (
        <div className="br-ui-faint br-ui-break">
          {el.css}
        </div>
      ) : null}
    </div>
  )
}

/* ---------------- describe ---------------- */

export function DescribeView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const req = safeParseRequest(describeRequestSchema, input)
  if (!req) return null
  const chips = (
    <FilterChip label={req.kind ?? 'css'} value={req.query ?? ''} />
  )

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <Badge variant="default">describing…</Badge>
          {chips}
        </MetaRow>
        <RunningNote label="locating element…" />
      </SectionShell>
    )
  }

  const res = safeParseResponse(describeResponseSchema, output)
  if (!res) return null
  if (!res.found || !res.element) {
    return (
      <SectionShell>
        <MetaRow>
          <Badge variant="warn">no match</Badge>
          {chips}
        </MetaRow>
        <EmptyState title="Element not found" description="Nothing matched the query." />
      </SectionShell>
    )
  }

  const el = res.element
  const rows: Array<[string, string]> = [
    ['tag', el.tag ?? ''],
    ['css', el.css ?? ''],
    ['full css', el.full_css ?? ''],
    ['xpath', el.xpath ?? ''],
    ['full xpath', el.full_xpath ?? ''],
    ['classes', (el.classes ?? []).join(' ') || '—'],
    ['parent', el.parent_tag ?? '—'],
    ['children', String(el.children ?? 0)],
    ['siblings', String(el.siblings ?? 0)],
  ]
  return (
    <SectionShell>
      <MetaRow>
        <Badge variant="accent">{el.tag ?? 'element'}</Badge>
        {chips}
      </MetaRow>
      {el.text ? (
        <div className="br-ui-text">
          {el.text}
        </div>
      ) : null}
      <TableViewport className="br-ui-table">
        <Table density="compact">
          <TableBody>
            {rows.map(([k, v]) => (
              <TableRow key={k}>
                <TableCell className="br-ui-faint br-ui-td-name">{k}</TableCell>
                <TableCell className="br-ui-break">{v}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableViewport>
      {el.attrs && Object.keys(el.attrs).length > 0 ? (
        <div>
          <div className={cn('br-ui-scrape-label', 'is-separated', uiClasses.eyebrow)}>
            attributes · {Object.keys(el.attrs).length}
          </div>
          <JsonHighlight code={JSON.stringify(el.attrs, null, 2)} wrap />
        </div>
      ) : null}
    </SectionShell>
  )
}
