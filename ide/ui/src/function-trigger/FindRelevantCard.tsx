import { ChevronDown, Sparkles } from 'lucide-react'
import { useId, useState } from 'react'
import {
  firstLocation,
  formatElapsed,
  ISSUE_LABELS,
  namedLeads,
  nextStep,
  previewLines,
  reasonLabel,
  type RelevantRow,
  type RelevantSummary,
} from './find-relevant'

const VISIBLE_FILE_LIMIT = 5
/** Rows mounted behind "Show more"; the rest are only counted. */
const OVERFLOW_FILE_LIMIT = 20
/** Rows that show their best excerpt inline: the answer, not the index. */
const PREVIEW_FILE_LIMIT = 2
const PREVIEW_LINE_LIMIT = 8
const LEAD_LIMIT = 4

export type OpenAt = (path: string, lineFrom?: number, lineTo?: number) => void

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`
}

function title(summary: RelevantSummary, running: boolean): string {
  if (running) return 'Asking the judge'
  if (summary.status === null) return 'Find relevant code'
  if (summary.status === 'unavailable') return 'Judge unavailable'
  if (summary.rows.length === 0) return 'No relevant files'
  return `Found ${plural(summary.rows.length, 'relevant file')}`
}

function meta(summary: RelevantSummary): string | null {
  if (summary.status === null || summary.status === 'unavailable') return null
  const parts = [plural(summary.judgeCalls, 'judge call'), formatElapsed(summary.elapsedMs)]
  if (summary.cacheHits > 0) parts.push(`${summary.cacheHits} cached`)
  return parts.join(' · ')
}

function countsLabel(row: RelevantRow): string {
  const parts: string[] = []
  if (row.excerpts.length) parts.push(plural(row.excerpts.length, 'excerpt'))
  if (row.leads.length) parts.push(plural(row.leads.length, 'lead'))
  if (row.sourceOmitted) parts.push('source omitted')
  return parts.join(' · ')
}

function rolesLabel(roles: readonly string[]): string {
  if (roles.length <= 2) return roles.join(' · ')
  return `${roles.slice(0, 2).join(' · ')} +${roles.length - 2}`
}

function issueList(issues: readonly [string, number][]): string {
  return issues
    .map(([kind, count]) => {
      const label = ISSUE_LABELS[kind] ?? kind
      return count > 1 && kind !== 'token_budget' ? `${label} ×${count}` : label
    })
    .join(', ')
}

/** Our own sentence for a partial result, with the issue counts and a
    next step for the reason it stopped. */
function partialNote(summary: RelevantSummary): string {
  const counted = issueList(summary.issues)
  if (summary.reason === 'token_budget') {
    const others = issueList(summary.issues.filter(([kind]) => kind !== 'token_budget'))
    const detail = others ? ` (${others})` : ''
    return `Stopped at the judge token budget${detail}: the folder was too big to judge in full. Ask about a narrower folder.`
  }
  const issues = counted || (summary.reason ? issueList([[summary.reason, 1]]) : '')
  const detail = issues ? ` (${issues})` : ''
  const next = nextStep(summary.reason ?? summary.issues[0]?.[0] ?? '')
  return `Partial result${detail}: some folders or files went unjudged.${next ? ` ${next}` : ''}`
}

function Note({ tone, children }: { tone: 'partial' | 'unavailable' | 'empty'; children: React.ReactNode }) {
  return <p className={`shui-relevant-note is-${tone}`}>{children}</p>
}

function Excerpt({ row, onOpen }: { row: RelevantRow; onOpen?: OpenAt }) {
  const excerpt = row.excerpts[0]
  const lines = previewLines(excerpt, PREVIEW_LINE_LIMIT)
  const range = `lines ${excerpt.lineFrom}–${excerpt.lineTo}`
  const hidden = excerpt.lineTo - excerpt.lineFrom + 1 - lines.length
  return (
    <figure className="shui-relevant-excerpt">
      <figcaption className="shui-relevant-excerpt-cap">
        {onOpen ? (
          <button
            type="button"
            className="shui-relevant-link"
            onClick={() => onOpen(row.path, excerpt.lineFrom, excerpt.lineTo)}
            title={`Open ${row.name} at ${range}`}
          >
            {range}
          </button>
        ) : (
          <span>{range}</span>
        )}
        {row.excerpts.length > 1 ? (
          <span className="t-faint">{`+${plural(row.excerpts.length - 1, 'excerpt')}`}</span>
        ) : null}
      </figcaption>
      <pre className="shui-relevant-code" data-clipped={hidden > 0 || undefined}>
        <code>
          {lines.map((line) => (
            <span key={line.number} className="shui-relevant-line">
              <span className="gutter" aria-hidden>
                {line.number}
              </span>
              <span className="text">{line.text || ' '}</span>
            </span>
          ))}
        </code>
      </pre>
    </figure>
  )
}

function Leads({ row, onOpen }: { row: RelevantRow; onOpen?: OpenAt }) {
  const named = namedLeads(row)
  if (named.length === 0) return null
  const shown = named.slice(0, LEAD_LIMIT)
  return (
    <ul className="shui-relevant-leads" aria-label={`Leads in ${row.name}`}>
      {shown.map((lead) => {
        const label = (
          <>
            <span className="name">{lead.name}</span>
            <span className="range">{`${lead.lineFrom}–${lead.lineTo}`}</span>
          </>
        )
        return (
          <li key={`${lead.name}:${lead.lineFrom}`}>
            {onOpen ? (
              <button
                type="button"
                className="shui-relevant-lead"
                onClick={() => onOpen(row.path, lead.lineFrom, lead.lineTo)}
                title={`Open ${lead.name} in ${row.name}`}
              >
                {label}
              </button>
            ) : (
              <span className="shui-relevant-lead">{label}</span>
            )}
          </li>
        )
      })}
      {named.length > shown.length ? (
        <li className="shui-relevant-leads-more">{`+${named.length - shown.length}`}</li>
      ) : null}
    </ul>
  )
}

function Row({ row, preview, onOpen }: { row: RelevantRow; preview: boolean; onOpen?: OpenAt }) {
  const at = firstLocation(row)
  const counts = countsLabel(row)
  const body = (
    <>
      <span
        className="shui-relevant-meter"
        style={{ '--rank': row.rank } as React.CSSProperties}
        aria-label={`relevance ${Math.round(row.rank * 100)}%`}
        role="img"
      />
      <span className="shui-relevant-path">
        {row.dir ? <span className="dir">{row.dir}</span> : null}
        <span className="name">{row.name}</span>
      </span>
      {row.roles.length ? <span className="shui-relevant-roles">{rolesLabel(row.roles)}</span> : null}
      {counts ? <span className="shui-relevant-counts">{counts}</span> : null}
    </>
  )
  return (
    <li className="shui-relevant-row" data-preview={preview || undefined}>
      {onOpen ? (
        <button
          type="button"
          className="shui-relevant-file"
          onClick={() => onOpen(row.path, at?.lineFrom, at?.lineTo)}
          title={at ? `Open ${row.path} at line ${at.lineFrom}` : `Open ${row.path}`}
        >
          {body}
        </button>
      ) : (
        <div className="shui-relevant-file">{body}</div>
      )}
      {preview ? (
        <div className="shui-relevant-detail">
          <Excerpt row={row} onOpen={onOpen} />
          <Leads row={row} onOpen={onOpen} />
        </div>
      ) : null}
    </li>
  )
}

function RowList({ rows, previewed, onOpen }: { rows: RelevantRow[]; previewed: Set<string>; onOpen?: OpenAt }) {
  return (
    <ol className="shui-relevant-list">
      {rows.map((row) => (
        <Row key={row.path} row={row} preview={previewed.has(row.path)} onOpen={onOpen} />
      ))}
    </ol>
  )
}

export function FindRelevantCard({
  summary,
  running,
  onOpen,
}: {
  summary: RelevantSummary
  running: boolean
  onOpen?: OpenAt
}) {
  const [expanded, setExpanded] = useState(false)
  const overflowId = useId()
  const primary = summary.rows.slice(0, VISIBLE_FILE_LIMIT)
  const overflow = summary.rows.slice(VISIBLE_FILE_LIMIT, VISIBLE_FILE_LIMIT + OVERFLOW_FILE_LIMIT)
  const unlisted = summary.rows.length - primary.length - overflow.length
  const previewed = new Set(
    primary
      .filter((row) => row.excerpts.length > 0)
      .slice(0, PREVIEW_FILE_LIMIT)
      .map((row) => row.path),
  )
  const heading = title(summary, running)
  const stats = running ? null : meta(summary)

  return (
    <section
      className="shui-relevant"
      aria-busy={running || undefined}
      data-state={running ? 'running' : 'settled'}
      data-status={summary.status ?? undefined}
    >
      <header className="shui-relevant-head">
        <Sparkles aria-hidden className="shui-relevant-icon" />
        <div className="shui-relevant-title-group">
          <h3 className="shui-relevant-title" role="status" aria-live="polite" aria-atomic="true">
            {heading}
          </h3>
          {stats ? <span className="shui-relevant-stats">{stats}</span> : null}
          {running ? <span className="shui-relevant-stats is-working">working…</span> : null}
        </div>
        {running ? <span className="shui-relevant-progress" aria-hidden /> : null}
      </header>

      <p className="shui-relevant-query">
        {summary.scope ? <span className="scope">{summary.scope.replace(/\/?$/, '/')}</span> : null}
        <q>{summary.query}</q>
      </p>

      {summary.status === 'unavailable' ? (
        <Note tone="unavailable">
          {summary.reason
            ? `The judge could not answer (${reasonLabel(summary.reason)}). ${nextStep(summary.reason)} `
            : 'The judge could not answer. '}
          Text search with <code>coder::search</code> still works.
        </Note>
      ) : null}
      {summary.status === 'incomplete' ? <Note tone="partial">{partialNote(summary)}</Note> : null}
      {summary.status === 'complete' && summary.rows.length === 0 ? (
        <Note tone="empty">The judge found nothing in this folder that answers the question.</Note>
      ) : null}

      {primary.length > 0 ? <RowList rows={primary} previewed={previewed} onOpen={onOpen} /> : null}

      {overflow.length > 0 ? (
        <>
          <div id={overflowId} className="shui-file-changes-overflow" data-open={expanded} aria-hidden={!expanded}>
            <div className="shui-file-changes-overflow-inner" inert={expanded ? undefined : true}>
              <RowList rows={overflow} previewed={previewed} onOpen={onOpen} />
            </div>
          </div>
          <button
            type="button"
            className="shui-file-changes-more"
            aria-expanded={expanded}
            aria-controls={overflowId}
            data-open={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            <span className="shui-file-changes-more-label">
              <span data-active={!expanded} aria-hidden={expanded}>
                Show {plural(overflow.length, 'more file', 'more files')}
              </span>
              <span data-active={expanded} aria-hidden={!expanded}>
                Show fewer files
              </span>
            </span>
            <ChevronDown aria-hidden />
          </button>
          {unlisted > 0 ? <Note tone="empty">{`+${plural(unlisted, 'lower-ranked file')} not listed`}</Note> : null}
        </>
      ) : null}
    </section>
  )
}
