import { JsonHighlight } from '@iii-dev/console-ui'
import { Chip, FilterChip, MetaRow, StatusPill } from '../../lib/shared'
import {
  extractRequestSchema,
  extractResponseSchema,
  findSimilarRequestSchema,
  findSimilarResponseSchema,
  formatChars,
  queryRequestSchema,
  queryResponseSchema,
  type SelectorSpec,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

const MAX_RESULT_ROWS = 50
const MAX_SIMILAR_ITEMS = 20

/** Smart Element Tracking marker — the match survives site redesigns via saved
 *  element identities. */
function AdaptiveChip({ domain }: { domain?: string }) {
  return (
    <Chip className="br-ui-scrape-adaptive">
      <span>adaptive</span>
      {domain ? (
        <span className="br-ui-scrape-chip-value">{domain}</span>
      ) : null}
    </Chip>
  )
}

function RunningNote({ label }: { label: string }) {
  return (
    <div className="br-ui-scrape-running">
      · {label}
    </div>
  )
}

function SectionShell({ children }: { children: React.ReactNode }) {
  return <div className="br-ui-scrape-section">{children}</div>
}

/* ---------------- browser::extract ---------------- */

function selectorSummary(spec: SelectorSpec): string {
  const query = spec.css
    ? `css ${spec.css}`
    : spec.xpath
      ? `xpath ${spec.xpath}`
      : spec.regex
        ? `re ${spec.regex}`
        : '—'
  const mods = [
    spec.attr ? `attr=${spec.attr}` : null,
    spec.html ? 'html' : null,
    spec.all ? 'all' : null,
  ]
    .filter(Boolean)
    .join(' · ')
  return mods ? `${query} · ${mods}` : query
}

function SelectorRows({ selectors }: { selectors: SelectorSpec[] }) {
  return (
    <div className="br-ui-scrape-selector-list">
      {selectors.map((spec, i) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; specs never reorder and names may repeat
          key={`${i}:${spec.name}`}
          className="br-ui-scrape-selector-row"
        >
          <span className="br-ui-scrape-selector-name">{spec.name}</span>
          <span className="br-ui-scrape-selector-value">
            ← {selectorSummary(spec)}
          </span>
        </div>
      ))}
    </div>
  )
}

export function ExtractView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const req = safeParseRequest(extractRequestSchema, input)
  if (!req) return null
  const chips = (
    <>
      {req.selectors?.length ? (
        <FilterChip label="selectors" value={req.selectors.length} />
      ) : null}
      {req.adaptive ? <AdaptiveChip domain={req.adaptive_domain} /> : null}
      {req.html != null ? (
        <FilterChip label="html" value={formatChars(req.html.length)} />
      ) : null}
    </>
  )

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="extracting…" variant="default" />
          {chips}
        </MetaRow>
        <RunningNote label="parsing…" />
      </SectionShell>
    )
  }

  const res = safeParseResponse(extractResponseSchema, output)
  if (!res) return null
  const fields = Object.keys(res.extracted).length
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={`${fields} field${fields === 1 ? '' : 's'}`}
          variant={fields ? 'accent' : 'warn'}
        />
        {chips}
      </MetaRow>
      {req.selectors?.length ? (
        <SelectorRows selectors={req.selectors} />
      ) : null}
      <JsonHighlight code={JSON.stringify(res.extracted, null, 2)} wrap />
    </SectionShell>
  )
}

/* ---------------- browser::css / xpath / regex ---------------- */

export function QueryView({
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
  const req = safeParseRequest(queryRequestSchema, input)
  if (!req) return null
  const op = functionId.slice('browser::'.length)
  const query = op === 'regex' ? req.pattern : req.query
  if (query == null) return null
  const chips = (
    <>
      <FilterChip label={op === 'regex' ? 'pattern' : op} value={query} />
      {req.attr ? <FilterChip label="attr" value={req.attr} /> : null}
      {req.first ? <Chip>first</Chip> : null}
      {req.adaptive ? <AdaptiveChip domain={req.adaptive_domain} /> : null}
      {req.html != null ? (
        <FilterChip label="html" value={formatChars(req.html.length)} />
      ) : null}
    </>
  )

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label={`${op}…`} variant="default" />
          {chips}
        </MetaRow>
        <RunningNote label="querying…" />
      </SectionShell>
    )
  }

  const res = safeParseResponse(queryResponseSchema, output)
  if (!res) return null
  const matches =
    res.result == null ? 0 : Array.isArray(res.result) ? res.result.length : 1
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={
            matches === 0
              ? 'no match'
              : `${matches} match${matches === 1 ? '' : 'es'}`
          }
          variant={matches ? 'accent' : 'warn'}
        />
        {chips}
      </MetaRow>
      <ResultRows result={res.result} />
    </SectionShell>
  )
}

function ResultRows({ result }: { result: string | (string | null)[] | null }) {
  if (result == null || (Array.isArray(result) && result.length === 0)) {
    return (
      <div className="br-ui-scrape-empty">
        · no match
      </div>
    )
  }
  const numbered = Array.isArray(result)
  const rows = numbered ? result : [result]
  return (
    <div>
      {rows.slice(0, MAX_RESULT_ROWS).map((row, i) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: static wire snapshot; matches never reorder and often repeat
          key={`${i}:${row ?? ''}`}
          className="br-ui-scrape-result-row"
        >
          {numbered ? (
            <span className="br-ui-scrape-result-number">
              {i + 1}
            </span>
          ) : null}
          {row == null ? (
            <span className="br-ui-scrape-dim">∅</span>
          ) : (
            <span className="br-ui-scrape-row-main br-ui-scrape-prewrap">
              {row}
            </span>
          )}
        </div>
      ))}
      {rows.length > MAX_RESULT_ROWS ? (
        <div className="br-ui-scrape-more">
          +{rows.length - MAX_RESULT_ROWS} more
        </div>
      ) : null}
    </div>
  )
}

/* ---------------- browser::find-similar ---------------- */

export function FindSimilarView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const req = safeParseRequest(findSimilarRequestSchema, input)
  if (!req) return null
  const chips = (
    <>
      {req.anchor ? <FilterChip label="anchor" value={req.anchor} /> : null}
      {typeof req.similarity_threshold === 'number' ? (
        <FilterChip label="threshold" value={req.similarity_threshold} />
      ) : null}
      {req.match_text ? <Chip>match text</Chip> : null}
      {req.html != null ? (
        <FilterChip label="html" value={formatChars(req.html.length)} />
      ) : null}
    </>
  )

  if (running) {
    return (
      <SectionShell>
        <MetaRow>
          <StatusPill label="matching…" variant="default" />
          {chips}
        </MetaRow>
        <RunningNote label="scanning structure…" />
      </SectionShell>
    )
  }

  const res = safeParseResponse(findSimilarResponseSchema, output)
  if (!res) return null
  const shown = res.items.slice(0, MAX_SIMILAR_ITEMS)
  return (
    <SectionShell>
      <MetaRow>
        <StatusPill
          label={`${res.count} similar`}
          variant={res.count ? 'accent' : 'warn'}
        />
        {chips}
      </MetaRow>
      {res.count === 0 ? (
        <div className="br-ui-scrape-empty">
          · no similar elements
        </div>
      ) : (
        <>
          <JsonHighlight code={JSON.stringify(shown, null, 2)} wrap />
          {res.items.length > MAX_SIMILAR_ITEMS ? (
            <div className="br-ui-scrape-more is-separated">
              +{res.items.length - MAX_SIMILAR_ITEMS} more items
            </div>
          ) : null}
        </>
      )}
    </SectionShell>
  )
}
