/** `sandbox::fs::sed` — per-file replacement table with the total pill. */

import {
  Badge,
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@iii-dev/console-ui'
import { TriangleAlert } from 'lucide-react'
import { type FsSedFileResult, fsSedRequestSchema, fsSedResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

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
        <Chip label="replacement">{req.data.replacement || "''"}</Chip>
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
    <Table density="compact" inset>
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
            <TableCell>{r.path}</TableCell>
            <TableCell className="faint num">{r.replacements}</TableCell>
            <TableCell>
              {r.success ? (
                <span className="cr-fam-accent">ok</span>
              ) : r.error ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    {/* A real button so keyboard users can reach the
                        tooltip; the class carries the reset. */}
                    <button type="button" className="cr-fam-sed-err" aria-label={`error for ${r.path}: ${r.error}`}>
                      <TriangleAlert size={16} aria-hidden /> err
                    </button>
                  </TooltipTrigger>
                  <TooltipContent>{r.error}</TooltipContent>
                </Tooltip>
              ) : (
                <span className="cr-fam-warn">err</span>
              )}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
      <TableFooter>
        <TableRow>
          <TableCell className="faint">total</TableCell>
          <TableCell colSpan={2}>
            <Badge variant={totalReplacements > 0 ? 'ok' : 'default'}>{`${totalReplacements} replacements`}</Badge>
          </TableCell>
        </TableRow>
      </TableFooter>
    </Table>
  )
}
