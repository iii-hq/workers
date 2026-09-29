/* shell::fs::grep and shell::fs::sed — match list + replacement table. */

import {
  Badge,
  Chip,
  EmptyState,
  MetaRow,
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
  Tooltip,
} from '@iii-dev/console-ui'
import { truncateMiddle } from '../lib/format'
import { renderWithHighlight } from '../lib/highlight'
import {
  type FsMatch,
  type FsSedFileResult,
  fsGrepRequestSchema,
  fsGrepResponseSchema,
  fsSedRequestSchema,
  fsSedResponseSchema,
  safeParseResponse,
} from './parsers'
import { items, kv, targetItem } from './shared'

interface ViewProps {
  input: unknown
  output: unknown
}

/* ---------------- fs::grep ---------------- */

export function FsGrepView({ input, output }: ViewProps) {
  const req = fsGrepRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsGrepResponseSchema, output)
  if (!resp) return null
  const { matches, truncated } = resp

  return (
    <div className="shui-card">
      <MetaRow
        items={items(
          kv('path', req.data.path),
          kv('pattern', req.data.pattern),
          targetItem(req.data.target),
          req.data.include_glob?.length ? kv('include', truncateMiddle(req.data.include_glob.join(' '), 40)) : null,
          req.data.exclude_glob?.length ? kv('exclude', truncateMiddle(req.data.exclude_glob.join(' '), 40)) : null,
        )}
      >
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
        {/* `recursive` defaults TRUE on the wire — chip the deviation only */}
        {req.data.recursive === false ? <Chip>non-recursive</Chip> : null}
        <Badge variant={matches.length > 0 ? 'default' : 'warn'}>
          {`${matches.length} ${matches.length === 1 ? 'match' : 'matches'}`}
        </Badge>
        {truncated ? <Badge variant="warn">truncated</Badge> : null}
      </MetaRow>

      {matches.length === 0 ? (
        <div className="shui-card-body">
          <EmptyState title="No matches" description="Nothing in the searched files matched the pattern." />
        </div>
      ) : (
        <GrepMatchList
          matches={matches}
          pattern={req.data.pattern}
          ignoreCase={!!req.data.ignore_case}
        />
      )}
    </div>
  )
}

interface GrepMatchListProps {
  matches: FsMatch[]
  pattern: string
  ignoreCase: boolean
}

/** Highlighted match list — grep patterns are always regexes on the wire. */
export function GrepMatchList({
  matches,
  pattern,
  ignoreCase,
}: GrepMatchListProps) {
  return (
    <div className="shui-match-list">
      {matches.map((m) => (
        <div
          /* path+line+content is collision-resistant in practice; React
             only warns if the worker ever sends true duplicates, which
             would itself be a wire-shape bug worth surfacing. */
          key={`${m.path}:${m.line}:${m.content}`}
          className="shui-match"
        >
          <div className="loc">
            <span className="t-accent">{m.path}</span>
            <span className="t-ghost">:</span>
            <span className="num">{m.line}</span>
          </div>
          <pre className="shui-pre">
            <code>
              {renderWithHighlight(m.content, pattern, {
                isRegex: true,
                ignoreCase,
              })}
            </code>
          </pre>
        </div>
      ))}
    </div>
  )
}

/* ---------------- fs::sed ---------------- */

export function FsSedView({ input, output }: ViewProps) {
  const req = fsSedRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsSedResponseSchema, output)
  if (!resp) return null
  const { results, total_replacements } = resp
  const pathMode = req.data.path != null
  const target =
    req.data.path ??
    (req.data.files?.length ? `${req.data.files.length} files` : '—')

  return (
    <div className="shui-card">
      <MetaRow
        items={items(
          kv('target', target),
          kv('pattern', req.data.pattern),
          kv('→', req.data.replacement || "''"),
          req.data.include_glob?.length ? kv('include', truncateMiddle(req.data.include_glob.join(' '), 40)) : null,
          req.data.exclude_glob?.length ? kv('exclude', truncateMiddle(req.data.exclude_glob.join(' '), 40)) : null,
          targetItem(req.data.target),
        )}
      >
        {/* `regex`/`recursive` default TRUE on the wire — chip the
            deviations ("literal", "non-recursive") only */}
        {req.data.regex === false ? <Chip>literal</Chip> : null}
        {req.data.first_only ? <Chip>first-only</Chip> : null}
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
        {pathMode && req.data.recursive === false ? <Chip>non-recursive</Chip> : null}
      </MetaRow>

      {results.length === 0 ? (
        <div className="shui-card-body">
          <EmptyState title="No files touched" description="No file contained the pattern." />
        </div>
      ) : (
        <SedResultsTable
          results={results}
          totalReplacements={total_replacements}
        />
      )}
    </div>
  )
}

interface SedResultsTableProps {
  results: FsSedFileResult[]
  totalReplacements: number
}

/** Per-file replacement table with the total pill. */
export function SedResultsTable({
  results,
  totalReplacements,
}: SedResultsTableProps) {
  return (
    <Table density="compact">
      <TableHeader>
        <TableRow>
          <TableHead>path</TableHead>
          <TableHead className="num">replacements</TableHead>
          <TableHead>status</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {results.map((r) => (
          <TableRow key={r.path}>
            <TableCell className="shui-path t-ink">{r.path}</TableCell>
            <TableCell className="t-faint num">{r.replacements}</TableCell>
            <TableCell>
              {r.success ? (
                <span className="t-accent">ok</span>
              ) : r.error ? (
                <Tooltip label={r.error}>
                  <Badge variant="warn">err</Badge>
                </Tooltip>
              ) : (
                <span className="t-warn">err</span>
              )}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
      <TableFooter>
        <TableRow>
          <TableCell className="t-faint">total</TableCell>
          <TableCell colSpan={2}>
            <Badge variant={totalReplacements > 0 ? 'accent' : 'default'}>{`${totalReplacements} replacements`}</Badge>
          </TableCell>
        </TableRow>
      </TableFooter>
    </Table>
  )
}
