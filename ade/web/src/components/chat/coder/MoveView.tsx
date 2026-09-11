/**
 * `coder::move` — batched `from → to` summary (no file body on wire).
 *
 * success + !moved is a no-op self-move (from and to resolve to the same
 * file) — rendered "unchanged", never as a completed move. Failed entries
 * also print the full WireError message inline: cross-root rollback
 * failures name BOTH leftover states (copy/source) for manual cleanup,
 * and C210 destination-is-directory messages carry a corrected target
 * path — neither belongs hidden in a tooltip alone.
 */
import { TriangleAlert } from 'lucide-react'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@/components/ui/Table'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { OpenInEditorButton } from './OpenInEditorButton'
import {
  moveFileRequestSchema,
  moveFileResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface MoveViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

export function MoveView({ input, output, running, preview }: MoveViewProps) {
  const req = safeParseRequest(moveFileRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(moveFileResponseSchema, output)
      : null

  const pending = preview || running

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{req.files.length}</Chip>
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · moving…
          </span>
        ) : null}
      </div>

      <TableViewport>
        <TableFrame className="px-3">
          <Table density="compact">
            <TableHeader>
              <TableRow>
                <TableHead>From → To</TableHead>
                <TableHead className="w-28">Outcome</TableHead>
                <TableHead className="w-16">Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {req.files.map((spec, i) => {
                const result = resp?.results[i]
                // Canonical absolute paths once the wire responds (caller's
                // input verbatim when resolution failed); request paths until.
                const from = result?.from ?? spec.from
                const to = result?.to ?? spec.to
                const outcome = pending
                  ? { label: 'Pending', tone: 'text-ink-faint' }
                  : !result
                    ? { label: '—', tone: 'text-ink-faint' }
                    : result.moved
                      ? { label: 'Moved', tone: 'text-ink' }
                      : result.success
                        ? { label: 'Unchanged', tone: 'text-ink-ghost' }
                        : { label: 'Failed', tone: 'text-ink-ghost' }

                return (
                  <TableRow key={`${spec.from}→${spec.to}`}>
                    <TableCell>
                      <div className="flex flex-wrap items-baseline gap-2 font-code">
                        <span>{from}</span>
                        <span className="text-ink-ghost">→</span>
                        <span>{to}</span>
                        {!pending && result?.success ? (
                          <OpenInEditorButton path={result.to} />
                        ) : null}
                        {spec.overwrite ? (
                          <Chip
                            label="overwrite"
                            className="border-warn text-warn"
                          >
                            true
                          </Chip>
                        ) : null}
                      </div>
                      {result?.error ? (
                        <div className="mt-1 break-words text-[11px] text-warn">
                          {result.error.message}
                        </div>
                      ) : null}
                    </TableCell>
                    <TableCell>
                      <span className={outcome.tone}>{outcome.label}</span>
                    </TableCell>
                    <TableCell>
                      {pending || !result ? (
                        <span className="text-ink-faint">—</span>
                      ) : result.success ? (
                        <span className="text-accent">OK</span>
                      ) : result.error ? (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <span className="inline-flex cursor-help items-center gap-1 text-warn">
                              <TriangleAlert aria-hidden className="size-4" />
                              {result.error.code}
                            </span>
                          </TooltipTrigger>
                          <TooltipContent>
                            {result.error.message}
                          </TooltipContent>
                        </Tooltip>
                      ) : (
                        <span className="text-warn">Error</span>
                      )}
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>
    </div>
  )
}

export function MovePreview({ input }: { input: unknown }) {
  return <MoveView input={input} preview />
}
