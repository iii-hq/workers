/**
 * Per-kind renderers for a `github::called` event's budgeted `result_preview`
 * (see the `preview()` budgeter in github/src/events.rs). The worker tags each
 * event with a `kind` discriminator so the feed can pick a renderer without
 * re-parsing the payload:
 *
 * - `list`    → a compact table of the projected items (PRs/issues as
 *   `#num title` + state Badge + author; api contents as `type name (size)`;
 *   everything else as a column table of the salient keys) + a "showing N of M"
 *   footer when the worker truncated the list.
 * - `object`  → the projected object as highlighted JSON (`JsonHighlight`).
 * - `text`    → a monospace block (also the shape errors arrive in).
 * - `diff`    → monospace with +/- line coloring off `--color-ok`/`--color-alert`.
 * - `outcome` → the exit line plus byte-capped stdout/stderr streams.
 *
 * Every renderer degrades to a muted empty note rather than throwing, and the
 * page wraps the whole thing in the console's ErrorBoundary, so a surprising
 * payload can never break the feed.
 */

import { Badge, type BadgeProps, JsonHighlight } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'

/** The renderer keys the worker emits; unknown values fall back to `object`. */
export type PreviewKind = 'list' | 'object' | 'text' | 'diff' | 'outcome'

/** Render one event's `result_preview` by its `kind`. `ok` tints an error
 *  text block (errors arrive as `kind: "text"`). */
export function ResultView({ kind, preview, ok }: { kind: string; preview: unknown; ok: boolean }): ReactNode {
  switch (kind) {
    case 'list':
      return <ListView preview={preview} />
    case 'text':
      return <TextView preview={preview} error={!ok} />
    case 'diff':
      return <DiffView preview={preview} />
    case 'outcome':
      return <OutcomeView preview={preview} />
    default:
      return <ObjectView preview={preview} />
  }
}

/* ── list ──────────────────────────────────────────────────────────────── */

function ListView({ preview }: { preview: unknown }) {
  const rec = asRecord(preview)
  const items = Array.isArray(rec?.items) ? (rec.items as unknown[]) : []
  const total = typeof rec?.total === 'number' ? (rec.total as number) : items.length
  if (items.length === 0) return <Empty>no items</Empty>

  const shape = listShape(items[0])
  return (
    <div className="gh-rv">
      <table className="gh-rv-list">
        <tbody>
          {shape === 'generic' ? (
            <GenericRows items={items} />
          ) : (
            items.map((item, i) => <ListRow key={i} item={item} shape={shape} />)
          )}
        </tbody>
      </table>
      {items.length < total ? (
        <div className="gh-rv-more">
          showing {items.length} of {total}
        </div>
      ) : null}
    </div>
  )
}

type Shape = 'issue' | 'contents' | 'generic'

/** Classify a list by the SHAPE of its projected items so the renderer works
 *  for both allowlisted and default-projected functions (see keys_for in
 *  events.rs). `number`+`title` ⇒ a PR/issue; `type`+`name`/`path` ⇒ api
 *  contents; anything else ⇒ a generic column table. */
function listShape(item: unknown): Shape {
  const r = asRecord(item)
  if (!r) return 'generic'
  if ('number' in r && 'title' in r) return 'issue'
  if ('type' in r && ('name' in r || 'path' in r)) return 'contents'
  return 'generic'
}

function ListRow({ item, shape }: { item: unknown; shape: Shape }) {
  const r = asRecord(item)
  if (!r) return null
  return shape === 'issue' ? <IssueRow r={r} /> : <ContentsRow r={r} />
}

function IssueRow({ r }: { r: Record<string, unknown> }) {
  const num = r.number
  const title = str(r.title)
  const state = str(r.state)
  const author = authorLogin(r.author)
  const repo = repoName(r.repository)
  const url = typeof r.url === 'string' ? r.url : undefined
  const draft = r.isDraft === true
  return (
    <tr className="gh-rv-row">
      <td className="gh-rv-num">#{num as ReactNode}</td>
      <td className="gh-rv-title">
        {url ? (
          <a href={url} target="_blank" rel="noreferrer">
            {title}
          </a>
        ) : (
          title
        )}
        {draft ? <span className="gh-rv-flag">draft</span> : null}
      </td>
      <td className="gh-rv-state">
        {state ? <Badge variant={stateVariant(state)}>{state.toLowerCase()}</Badge> : null}
      </td>
      <td className="gh-rv-author">
        {author ? <span>@{author}</span> : null}
        {repo ? <span className="gh-rv-repo">{repo}</span> : null}
      </td>
    </tr>
  )
}

function ContentsRow({ r }: { r: Record<string, unknown> }) {
  const type = str(r.type)
  const name = str(r.name) || str(r.path)
  const size = typeof r.size === 'number' ? (r.size as number) : undefined
  return (
    <tr className="gh-rv-row">
      <td className="gh-rv-ctype">{type ? <Badge>{type}</Badge> : null}</td>
      <td className="gh-rv-cname">{name}</td>
      <td className="gh-rv-csize">{size != null ? humanBytes(size) : ''}</td>
    </tr>
  )
}

/** A column table for lists with no bespoke shape (repos, runs, releases,
 *  workflows, code search): columns are the projected scalar keys of the first
 *  item, values coerced to a short display string. */
function GenericRows({ items }: { items: unknown[] }) {
  const cols = scalarKeys(items[0])
  if (cols.length === 0) {
    return (
      <tr>
        <td>
          <ObjectView preview={items} />
        </td>
      </tr>
    )
  }
  return (
    <>
      <tr className="gh-rv-ghead">
        {cols.map((c) => (
          <th key={c}>{c}</th>
        ))}
      </tr>
      {items.map((item, i) => {
        const r = asRecord(item)
        return (
          <tr key={i} className="gh-rv-row">
            {cols.map((c) => (
              <td key={c} className="gh-rv-gcell">
                {cellText(r ? r[c] : undefined)}
              </td>
            ))}
          </tr>
        )
      })}
    </>
  )
}

/* ── object / text / diff / outcome ────────────────────────────────────── */

function ObjectView({ preview }: { preview: unknown }) {
  if (preview == null) return <Empty>no result</Empty>
  return <JsonHighlight code={JSON.stringify(preview, null, 2)} wrap />
}

function TextView({ preview, error }: { preview: unknown; error: boolean }) {
  const r = asRecord(preview)
  const text = typeof r?.text === 'string' ? (r.text as string) : typeof preview === 'string' ? preview : ''
  const truncated = r?.truncated === true
  if (!text) return <Empty>{error ? 'error' : 'no output'}</Empty>
  return (
    <div className={`gh-rv-text${error ? ' gh-rv-errtext' : ''}`}>
      {text}
      {truncated ? <TruncMark /> : null}
    </div>
  )
}

function DiffView({ preview }: { preview: unknown }) {
  const r = asRecord(preview)
  const diff = typeof r?.diff === 'string' ? (r.diff as string) : ''
  const truncated = r?.truncated === true
  if (!diff) return <Empty>no diff</Empty>
  const lines = diff.split('\n')
  return (
    <div className="gh-rv-diff">
      {lines.map((line, i) => (
        <div key={i} className={`gh-rv-dline${diffClass(line)}`}>
          {line || ' '}
        </div>
      ))}
      {truncated ? <TruncMark /> : null}
    </div>
  )
}

function diffClass(line: string): string {
  if (line.startsWith('+') && !line.startsWith('+++')) return ' gh-rv-add'
  if (line.startsWith('-') && !line.startsWith('---')) return ' gh-rv-del'
  if (line.startsWith('@@')) return ' gh-rv-hunk'
  return ''
}

function OutcomeView({ preview }: { preview: unknown }) {
  const r = asRecord(preview)
  if (!r) return <Empty>no outcome</Empty>
  const exit = r.exit_code
  const timedOut = r.timed_out === true
  const stdout = typeof r.stdout === 'string' ? (r.stdout as string) : ''
  const stderr = typeof r.stderr === 'string' ? (r.stderr as string) : ''
  const truncated = r.truncated === true
  const label = timedOut ? 'timed out' : exit == null ? 'killed' : `exit ${exit as ReactNode}`
  const failed = timedOut || exit == null || (typeof exit === 'number' && exit !== 0)
  return (
    <div className="gh-rv-outcome">
      <div className={`gh-rv-exit${failed ? ' gh-rv-errtext' : ''}`}>{label}</div>
      {stdout ? (
        <div className="gh-rv-stream">
          <div className="gh-rv-slabel">stdout</div>
          <div className="gh-rv-text">{stdout}</div>
        </div>
      ) : null}
      {stderr ? (
        <div className="gh-rv-stream">
          <div className="gh-rv-slabel">stderr</div>
          <div className="gh-rv-text gh-rv-errtext">{stderr}</div>
        </div>
      ) : null}
      {truncated ? <TruncMark /> : null}
    </div>
  )
}

/* ── small helpers ─────────────────────────────────────────────────────── */

function Empty({ children }: { children: ReactNode }) {
  return <div className="gh-rv-empty">{children}</div>
}

function TruncMark() {
  return <div className="gh-rv-trunc">… truncated</div>
}

function asRecord(v: unknown): Record<string, unknown> | null {
  return v != null && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null
}

function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

/** The `login` of a projected `author`/`user` object, or a bare string author. */
function authorLogin(a: unknown): string {
  const r = asRecord(a)
  if (r && typeof r.login === 'string') return r.login
  return typeof a === 'string' ? a : ''
}

/** A `repository` field can be a bare `owner/name` string or a projected
 *  `{ nameWithOwner | name }` object (gh search results). */
function repoName(v: unknown): string {
  if (typeof v === 'string') return v
  const r = asRecord(v)
  if (r) return str(r.nameWithOwner) || str(r.name)
  return ''
}

/** Scalar (non-object, non-array) keys of a projected item, in insertion
 *  order — the columns for the generic table. */
function scalarKeys(item: unknown): string[] {
  const r = asRecord(item)
  if (!r) return []
  return Object.keys(r).filter((k) => {
    const v = r[k]
    return v == null || typeof v !== 'object'
  })
}

/** Coerce a projected value to a short display string: objects collapse to a
 *  name-ish field, arrays join, everything else stringifies. */
function cellText(v: unknown): string {
  if (v == null) return ''
  if (typeof v === 'string') return v
  if (typeof v === 'number' || typeof v === 'boolean') return String(v)
  if (Array.isArray(v)) return v.map(cellText).join(', ')
  const r = asRecord(v)
  if (r) return str(r.login) || str(r.nameWithOwner) || str(r.name) || JSON.stringify(v)
  return JSON.stringify(v)
}

/** State → Badge tone: open/merged read as active, closed as spent. */
function stateVariant(state: string): NonNullable<BadgeProps['variant']> {
  const s = state.toUpperCase()
  if (s === 'OPEN' || s === 'MERGED') return 'accent'
  if (s === 'CLOSED') return 'alert'
  return 'default'
}

function humanBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / (1024 * 1024)).toFixed(1)} MB`
}
