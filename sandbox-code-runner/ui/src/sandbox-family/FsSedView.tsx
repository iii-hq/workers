/** `sandbox::fs::sed` — per-file replacement table with the total pill. */

import { Tooltip, TooltipContent, TooltipTrigger } from '@iii-dev/console-ui'
import { type FsSedFileResult, fsSedRequestSchema, fsSedResponseSchema, safeParseResponse } from './parsers'
import { Chip, FooterPill, SandboxIdChip } from './shared'

interface FsSedViewProps {
  input: unknown
  output: unknown
}

export function FsSedView({ input, output }: FsSedViewProps) {
  const req = fsSedRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsSedResponseSchema, output)
  if (!resp) return null
  const { results, total_replacements } = resp
  const target = req.data.path ?? (req.data.files?.length ? `${req.data.files.length} files` : '—')

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-chips-row">
        <SandboxIdChip sandboxId={req.data.sandbox_id} />
        <Chip label="target">{target}</Chip>
        <Chip label="pattern">{req.data.pattern}</Chip>
        <Chip label="→">{req.data.replacement || "''"}</Chip>
        {req.data.regex === false ? <Chip>literal</Chip> : null}
        {req.data.first_only ? <Chip>first-only</Chip> : null}
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
      </div>

      {results.length === 0 ? (
        <div className="cr-fam-note-ghost">· no files touched</div>
      ) : (
        <SedResultsTable results={results} totalReplacements={total_replacements} />
      )}
    </div>
  )
}

interface SedResultsTableProps {
  results: FsSedFileResult[]
  totalReplacements: number
}

/** Per-file replacement table — the wire speaks `FsSedFileResult`. */
function SedResultsTable({ results, totalReplacements }: SedResultsTableProps) {
  return (
    <table className="cr-fam-table">
      <thead>
        <tr>
          <th>path</th>
          <th className="num">replacements</th>
          <th>status</th>
        </tr>
      </thead>
      <tbody>
        {results.map((r) => (
          <tr key={r.path}>
            <td>{r.path}</td>
            <td className="faint num">{r.replacements}</td>
            <td>
              {r.success ? (
                <span className="cr-fam-accent">ok</span>
              ) : r.error ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    {/* A real button so keyboard users can reach the
                        tooltip; the class carries the reset. */}
                    <button type="button" className="cr-fam-sed-err" aria-label={`error for ${r.path}: ${r.error}`}>
                      ⚠ err
                    </button>
                  </TooltipTrigger>
                  <TooltipContent>{r.error}</TooltipContent>
                </Tooltip>
              ) : (
                <span className="cr-fam-warn">err</span>
              )}
            </td>
          </tr>
        ))}
        <tr className="total">
          <td className="faint label">total</td>
          <td colSpan={2}>
            <FooterPill tone={totalReplacements > 0 ? 'ok' : 'default'}>
              {`${totalReplacements} replacements`}
            </FooterPill>
          </td>
        </tr>
      </tbody>
    </table>
  )
}
