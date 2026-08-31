/* shell::fs::grep and shell::fs::sed — match list + replacement table. */

import { Tooltip, TooltipContent, TooltipTrigger } from '@iii-dev/console-ui'
import { TriangleAlert } from 'lucide-react'
import { truncateMiddle } from '../lib/format'
import { renderWithHighlight } from '../lib/highlight'
import { Chip, FooterPill } from '../lib/terminal'
import {
  type FsMatch,
  type FsSedFileResult,
  fsGrepRequestSchema,
  fsGrepResponseSchema,
  fsSedRequestSchema,
  fsSedResponseSchema,
  safeParseResponse,
} from './parsers'
import { TargetChip } from './shared'

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
      <div className="shui-head">
        <span className="shui-chips">
          <Chip label="path">{req.data.path}</Chip>
          <Chip label="pattern">{req.data.pattern}</Chip>
          <TargetChip target={req.data.target} />
          {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
          {/* `recursive` defaults TRUE on the wire — chip the deviation only */}
          {req.data.recursive === false ? <Chip>non-recursive</Chip> : null}
          {req.data.include_glob?.length ? (
            <Chip label="include">
              {truncateMiddle(req.data.include_glob.join(' '), 40)}
            </Chip>
          ) : null}
          {req.data.exclude_glob?.length ? (
            <Chip label="exclude">
              {truncateMiddle(req.data.exclude_glob.join(' '), 40)}
            </Chip>
          ) : null}
          <FooterPill tone={matches.length > 0 ? 'default' : 'warn'}>
            {`${matches.length} ${matches.length === 1 ? 'match' : 'matches'}`}
          </FooterPill>
          {truncated ? <FooterPill tone="warn">truncated</FooterPill> : null}
        </span>
      </div>

      {matches.length === 0 ? (
        <div className="shui-empty">· no matches</div>
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
          <pre className="shui-pre out">
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
      <div className="shui-head">
        <span className="shui-chips">
          <Chip label="target">{target}</Chip>
          <Chip label="pattern">{req.data.pattern}</Chip>
          <Chip label="→">{req.data.replacement || "''"}</Chip>
          {/* `regex`/`recursive` default TRUE on the wire — chip the
              deviations ("literal", "non-recursive") only */}
          {req.data.regex === false ? <Chip>literal</Chip> : null}
          {req.data.first_only ? <Chip>first-only</Chip> : null}
          {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
          {pathMode && req.data.recursive === false ? (
            <Chip>non-recursive</Chip>
          ) : null}
          {req.data.include_glob?.length ? (
            <Chip label="include">
              {truncateMiddle(req.data.include_glob.join(' '), 40)}
            </Chip>
          ) : null}
          {req.data.exclude_glob?.length ? (
            <Chip label="exclude">
              {truncateMiddle(req.data.exclude_glob.join(' '), 40)}
            </Chip>
          ) : null}
          <TargetChip target={req.data.target} />
        </span>
      </div>

      {results.length === 0 ? (
        <div className="shui-empty">· no files touched</div>
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
    <table className="shui-table">
      <thead>
        <tr>
          <th className="pad-l">path</th>
          <th className="num">replacements</th>
          <th className="pad-r">status</th>
        </tr>
      </thead>
      <tbody>
        {results.map((r) => (
          <tr key={r.path}>
            <td className="pad-l t-ink">{r.path}</td>
            <td className="t-faint num">{r.replacements}</td>
            <td className="pad-r">
              {r.success ? (
                <span className="t-accent">ok</span>
              ) : r.error ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="shui-err-chip">
                      <TriangleAlert aria-hidden className="shui-fs-icon" />
                      err
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{r.error}</TooltipContent>
                </Tooltip>
              ) : (
                <span className="t-warn">err</span>
              )}
            </td>
          </tr>
        ))}
        <tr className="total">
          <td className="pad-l label">total</td>
          <td colSpan={2}>
            <FooterPill tone={totalReplacements > 0 ? 'accent' : 'default'}>
              {`${totalReplacements} replacements`}
            </FooterPill>
          </td>
        </tr>
      </tbody>
    </table>
  )
}
