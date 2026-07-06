import { FilterChip } from '@/components/chat/engine/shared'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
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
  return <div className="border-t border-rule-2 bg-bg">{children}</div>
}

function RunningNote({ label }: { label: string }) {
  return (
    <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
      · {label}
    </div>
  )
}

/* ---------------- find / find-by-text / find-by-regex ---------------- */

function searchChips(functionId: string, input: unknown): React.ReactNode {
  if (functionId === 'scrapling::find') {
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
  if (functionId === 'scrapling::find-by-text') {
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
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no elements matched
        </div>
      ) : (
        <>
          {shown.map((el, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; rows never reorder and selectors may repeat
            <ElementRow key={`${i}:${el.css ?? ''}`} el={el} />
          ))}
          {res.items.length > MAX_ROWS ? (
            <div className="px-3 py-1.5 font-mono text-[11px] text-ink-ghost">
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
    <div className="px-3 py-1.5 border-b border-rule-2 last:border-b-0 flex flex-col gap-0.5">
      <div className="flex items-baseline gap-2 font-mono text-[12px]">
        {el.tag ? <span className="text-accent shrink-0">{el.tag}</span> : null}
        <span className="min-w-0 text-ink break-words">{el.text || '—'}</span>
      </div>
      {el.css ? (
        <div className="font-mono text-[11px] text-ink-faint break-all">
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
  const chips = req ? (
    <FilterChip label={req.kind ?? 'css'} value={req.query ?? ''} />
  ) : null

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
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
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
        <div className="px-3 py-2 border-b border-rule-2 font-mono text-[12.5px] text-ink break-words">
          {el.text}
        </div>
      ) : null}
      <table className="w-full font-mono text-[11.5px] text-ink">
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k} className="border-b border-rule-2 last:border-b-0">
              <td className="px-3 py-1 text-ink-faint align-top w-[26%] whitespace-nowrap">
                {k}
              </td>
              <td className="px-3 py-1 text-ink break-all">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {el.attrs && Object.keys(el.attrs).length > 0 ? (
        <div>
          <div className="px-3 py-1.5 border-y border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
            attributes · {Object.keys(el.attrs).length}
          </div>
          <JsonHighlight code={JSON.stringify(el.attrs, null, 2)} wrap />
        </div>
      ) : null}
    </SectionShell>
  )
}
