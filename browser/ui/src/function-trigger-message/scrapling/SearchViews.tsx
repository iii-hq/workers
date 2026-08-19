import { JsonHighlight } from '@iii-dev/console-ui'
import { Chip, FilterChip, MetaRow, StatusPill } from '../../lib/shared'
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
    <div className="br-ui-scrape-running">
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
          <StatusPill label="searching…" variant="default" />
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
        <StatusPill
          label={
            res.count === 0
              ? 'no match'
              : `${res.count} element${res.count === 1 ? '' : 's'}`
          }
          variant={res.count ? 'accent' : 'warn'}
        />
        {chips}
      </MetaRow>
      {res.count === 0 ? (
        <div className="br-ui-scrape-empty">
          · no elements matched
        </div>
      ) : (
        <>
          {shown.map((el, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; rows never reorder and selectors may repeat
            <ElementRow key={`${i}:${el.css ?? ''}`} el={el} />
          ))}
          {res.items.length > MAX_ROWS ? (
            <div className="br-ui-scrape-more">
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
    <div className="br-ui-scrape-element-row">
      <div className="br-ui-scrape-element-main">
        {el.tag ? (
          <span className="br-ui-scrape-element-tag">{el.tag}</span>
        ) : null}
        <span className="br-ui-scrape-row-main">{el.text || '—'}</span>
      </div>
      {el.css ? (
        <div className="br-ui-scrape-selector-value">
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
          <StatusPill label="describing…" variant="default" />
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
          <StatusPill label="no match" variant="warn" />
          {chips}
        </MetaRow>
        <div className="br-ui-scrape-empty">
          · element not found
        </div>
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
        <StatusPill label={el.tag ?? 'element'} variant="accent" />
        {chips}
      </MetaRow>
      {el.text ? (
        <div className="br-ui-scrape-description">
          {el.text}
        </div>
      ) : null}
      <div className="br-ui-scrape-table-wrap">
        <table className="br-ui-scrape-table">
          <tbody>
            {rows.map(([k, v]) => (
              <tr key={k}>
                <td className="br-ui-scrape-table-key">{k}</td>
                <td className="br-ui-scrape-table-value">{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {el.attrs && Object.keys(el.attrs).length > 0 ? (
        <div>
          <div className="br-ui-scrape-label is-separated">
            attributes · {Object.keys(el.attrs).length}
          </div>
          <JsonHighlight code={JSON.stringify(el.attrs, null, 2)} wrap />
        </div>
      ) : null}
    </SectionShell>
  )
}
