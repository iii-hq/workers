/**
 * `coder::search` — grep-style results (workers/coder/src/functions/search.rs).
 * Content matches grouped by file with a line-number gutter and dimmed
 * before/after context; path matches render as a second section. The
 * `truncated` flag is the headline honesty signal: the wire sets it on
 * the match cap OR the response byte budget, and the fix is to refine
 * the query / add include_globs — there is no pagination.
 */
import type { ReactNode } from 'react'
import { renderWithHighlight } from '@/components/chat/sandbox/highlight'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import { cn } from '@/lib/utils'
import {
  type ContentMatch,
  type PathMatch,
  type SearchRequest,
  safeParseRequest,
  safeParseResponse,
  searchRequestSchema,
  searchResponseSchema,
} from './parsers'

interface SearchViewProps {
  input: unknown
  output?: unknown
  running?: boolean
}

export function SearchView({ input, output, running }: SearchViewProps) {
  const req = safeParseRequest(searchRequestSchema, input)
  if (!req) return null
  const resp =
    output != null ? safeParseResponse(searchResponseSchema, output) : null

  const groups = resp ? groupContentMatches(resp.content_matches) : []
  const pathMatches = resp?.path_matches ?? []
  const hasMatches = groups.length > 0 || pathMatches.length > 0
  const isRegex = !!req.regex
  const ignoreCase = !!req.ignore_case

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="query">{`"${req.query}"`}</Chip>
        <RequestChips req={req} />
        {resp ? (
          <FooterPill tone={hasMatches ? 'default' : 'warn'}>
            {formatMatchCount(resp.content_matches.length, pathMatches.length)}
          </FooterPill>
        ) : null}
        {resp?.truncated ? (
          <FooterPill tone="warn">results truncated</FooterPill>
        ) : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · searching…
          </span>
        ) : null}
      </div>

      {resp?.truncated ? (
        <div className="border-b border-rule-2 px-3 py-1.5 font-mono text-[11px] text-warn">
          results truncated — refine the query or add include_globs rather than
          paginating
        </div>
      ) : null}

      {resp ? (
        hasMatches ? (
          <>
            {groups.length > 0 ? (
              <ContentSection
                groups={groups}
                query={req.query}
                isRegex={isRegex}
                ignoreCase={ignoreCase}
              />
            ) : null}
            {pathMatches.length > 0 ? (
              <PathSection
                paths={pathMatches}
                query={req.query}
                isRegex={isRegex}
                ignoreCase={ignoreCase}
                /* Single rule between the two sections — the content
                   section's last group drops its own border-b. */
                className={
                  groups.length > 0 ? 'border-t border-rule-2' : undefined
                }
              />
            ) : null}
          </>
        ) : (
          <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
            · no matches
          </div>
        )
      ) : null}
    </div>
  )
}

/** Request-derived honesty chips: every knob that narrowed (or widened)
    what the agent actually searched. */
function RequestChips({ req }: { req: SearchRequest }) {
  const context = formatContextRequest(
    req.context_lines_before,
    req.context_lines_after,
  )
  return (
    <>
      {req.regex ? <Chip>regex</Chip> : null}
      {req.ignore_case ? <Chip>case-insensitive</Chip> : null}
      {req.path ? <Chip label="path">{req.path}</Chip> : null}
      {req.include_globs?.length ? (
        <Chip label="include">{req.include_globs.join(' ')}</Chip>
      ) : null}
      {req.exclude_globs?.length ? (
        <Chip label="exclude">{req.exclude_globs.join(' ')}</Chip>
      ) : null}
      {/* Walking into .git / node_modules / target is unusual enough to
          warn-tint — results may include generated or vendored code. */}
      {req.use_default_excludes === false ? (
        <Chip className="border-warn text-warn">default excludes off</Chip>
      ) : null}
      {req.search_content === false ? <Chip>paths only</Chip> : null}
      {req.search_paths === false ? <Chip>content only</Chip> : null}
      {context ? <Chip label="context">{context}</Chip> : null}
      {req.max_matches != null ? (
        <Chip label="max">{req.max_matches}</Chip>
      ) : null}
    </>
  )
}

function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <div className="bg-paper-2 border-b border-rule-2 px-3 py-1 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
      {children}
    </div>
  )
}

interface HighlightOpts {
  query: string
  isRegex: boolean
  ignoreCase: boolean
}

function ContentSection({
  groups,
  query,
  isRegex,
  ignoreCase,
}: { groups: ContentGroup[] } & HighlightOpts) {
  return (
    <div className="font-mono text-[12px] leading-[1.55]">
      <SectionLabel>content matches</SectionLabel>
      {groups.map((group) => (
        <div
          key={group.path}
          className="border-b border-rule-2 last:border-b-0"
        >
          <div className="px-3 pt-1.5 text-accent break-all">{group.path}</div>
          {/* 3-col grid: line | column | text — auto-sized gutters keep
              line numbers aligned per file group. */}
          <div className="grid grid-cols-[auto_auto_1fr] px-3 pb-1.5">
            {buildGroupRows(group.matches).map((row) => (
              <MatchRowCells
                key={row.key}
                row={row}
                query={query}
                isRegex={isRegex}
                ignoreCase={ignoreCase}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}

function MatchRowCells({
  row,
  query,
  isRegex,
  ignoreCase,
}: { row: MatchRow } & HighlightOpts) {
  if (row.kind === 'gap') {
    return (
      <>
        <span className="text-right text-ink-ghost select-none">⋯</span>
        <span />
        <span />
      </>
    )
  }
  const isMatch = row.kind === 'match'
  return (
    <>
      {/* select-none gutters so copying a block yields just the code. */}
      <span
        className={cn(
          'text-right tabular-nums select-none',
          isMatch ? 'text-ink-faint' : 'text-ink-ghost',
        )}
      >
        {row.line}
      </span>
      <span className="text-ink-ghost tabular-nums select-none">
        {isMatch && row.column != null ? `:${row.column}` : ''}
      </span>
      <pre
        className={cn(
          'm-0 pl-2 whitespace-pre-wrap break-words',
          isMatch ? 'text-ink' : 'text-ink-faint',
        )}
      >
        <code>
          {isMatch
            ? renderWithHighlight(row.text, query, { isRegex, ignoreCase })
            : row.text}
        </code>
      </pre>
    </>
  )
}

function PathSection({
  paths,
  query,
  isRegex,
  ignoreCase,
  className,
}: { paths: PathMatch[]; className?: string } & HighlightOpts) {
  return (
    <div className={cn('font-mono text-[12px] leading-[1.55]', className)}>
      <SectionLabel>path matches</SectionLabel>
      {paths.map((p) => (
        <div
          key={p.path}
          className="border-b border-rule-2 last:border-b-0 px-3 py-1 text-accent break-all"
        >
          {renderWithHighlight(p.path, query, { isRegex, ignoreCase })}
        </div>
      ))}
    </div>
  )
}

/* ---------------- pure derivation helpers (unit-tested) ---------------- */

export interface ContentGroup {
  path: string
  matches: ContentMatch[]
}

/** Group content matches by file, preserving wire (walk) order for both
    files and the matches within each file. */
export function groupContentMatches(matches: ContentMatch[]): ContentGroup[] {
  const groups: ContentGroup[] = []
  const byPath = new Map<string, ContentGroup>()
  for (const m of matches) {
    const existing = byPath.get(m.path)
    if (existing) {
      existing.matches.push(m)
    } else {
      const group: ContentGroup = { path: m.path, matches: [m] }
      byPath.set(m.path, group)
      groups.push(group)
    }
  }
  return groups
}

export interface MatchRow {
  key: string
  kind: 'match' | 'context' | 'gap'
  /** 1-based file line; null for gap rows. */
  line: number | null
  /** Match column — only on `match` rows. */
  column?: number
  text: string
}

/**
 * Flatten one file's matches into display rows. Context line numbers are
 * derived by offset from the match line (`before` ends at line-1, `after`
 * starts at line+1 — the wire omits both arrays entirely when empty). A
 * `gap` row marks non-contiguous blocks so context lines from different
 * matches don't read as one continuous excerpt.
 */
export function buildGroupRows(matches: ContentMatch[]): MatchRow[] {
  const rows: MatchRow[] = []
  let prevEnd: number | null = null
  let ordinal = 0
  for (const m of matches) {
    const before = m.before ?? []
    const after = m.after ?? []
    const start = m.line - before.length
    if (prevEnd !== null && start > prevEnd + 1) {
      rows.push({ key: `gap:${ordinal++}`, kind: 'gap', line: null, text: '' })
    }
    before.forEach((text, i) => {
      rows.push({
        key: `ctx:${ordinal++}`,
        kind: 'context',
        line: start + i,
        text,
      })
    })
    rows.push({
      key: `match:${ordinal++}`,
      kind: 'match',
      line: m.line,
      column: m.column,
      text: m.text,
    })
    after.forEach((text, i) => {
      rows.push({
        key: `ctx:${ordinal++}`,
        kind: 'context',
        line: m.line + 1 + i,
        text,
      })
    })
    prevEnd = m.line + after.length
  }
  return rows
}

/** Header chip for requested context: `±2` when symmetric, else the
    grep-flavoured `-B +A` parts that were actually asked for. */
export function formatContextRequest(
  before?: number | null,
  after?: number | null,
): string | null {
  const b = before ?? 0
  const a = after ?? 0
  if (b === 0 && a === 0) return null
  if (b === a) return `±${b}`
  const parts: string[] = []
  if (b > 0) parts.push(`-${b}`)
  if (a > 0) parts.push(`+${a}`)
  return parts.join(' ')
}

/** Count pill text. Content hits are counted as "lines" on purpose —
    the wire reports at most ONE match per line, so a line with five
    hits is still a single ContentMatch. */
export function formatMatchCount(
  contentLines: number,
  pathHits: number,
): string {
  const parts: string[] = []
  if (contentLines > 0) {
    parts.push(`${contentLines} ${contentLines === 1 ? 'line' : 'lines'}`)
  }
  if (pathHits > 0) {
    parts.push(`${pathHits} ${pathHits === 1 ? 'path' : 'paths'}`)
  }
  if (parts.length === 0) return '0 matches'
  return parts.join(' · ')
}
