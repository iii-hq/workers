/**
 * `coder::delete-file` — path-level removal summary (no file body on wire).
 *
 * Missing paths are idempotent SUCCESSES: success + !removed renders as
 * "already absent", never as a deletion. Per-entry errors are structured
 * WireError {code, message}; C210 = refusing to delete an allowed root,
 * C211 = not-found-or-denied (incl. non-accessible entries mid-recursion).
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
import {
  deleteFileRequestSchema,
  deleteFileResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface DeleteFileViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

export function DeleteFileView({
  input,
  output,
  running,
  preview,
}: DeleteFileViewProps) {
  const req = safeParseRequest(deleteFileRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(deleteFileResponseSchema, output)
      : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="paths">{req.paths.length}</Chip>
        {req.recursive ? (
          <Chip label="recursive" className="border-warn text-warn">
            true
          </Chip>
        ) : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · removing…
          </span>
        ) : null}
      </div>

      <TableViewport>
        <TableFrame className="px-3">
          <Table density="compact">
            <TableHeader>
              <TableRow>
                <TableHead>Path</TableHead>
                <TableHead className="w-28">Outcome</TableHead>
                <TableHead className="w-16">Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {req.paths.map((path, i) => {
                const result = resp?.results[i]
                const pending = preview || running
                // success + !removed = idempotent no-op (path already gone).
                const outcome = pending
                  ? { label: 'Pending', tone: 'text-ink-faint' }
                  : !result
                    ? { label: '—', tone: 'text-ink-faint' }
                    : result.removed
                      ? { label: 'Removed', tone: 'text-warn' }
                      : result.success
                        ? { label: 'Already absent', tone: 'text-ink-ghost' }
                        : { label: 'Failed', tone: 'text-ink-ghost' }

                return (
                  <TableRow key={path}>
                    <TableCell className="font-code">{path}</TableCell>
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

export function DeleteFilePreview({ input }: { input: unknown }) {
  return <DeleteFileView input={input} preview />
}
