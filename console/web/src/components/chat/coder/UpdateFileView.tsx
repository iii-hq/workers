/**
 * `coder::update-file` — Pierre diff when the wire response includes
 * `before` / `after` snapshots; op-summary table for pending/running or
 * when snapshots are omitted (binary / oversize).
 */
import { TriangleAlert } from 'lucide-react'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { CoderFileDiff } from './CoderDiff'
import {
  formatUpdateOp,
  safeParseRequest,
  safeParseResponse,
  truncateInline,
  type UpdateFileRequest,
  type UpdateFileResponse,
  updateFileRequestSchema,
  updateFileResponseSchema,
} from './parsers'

interface UpdateFileViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  preview?: boolean
}

function hasDiffSnapshot(result: {
  success: boolean
  before?: string
  after?: string
}): result is { success: true; before: string; after: string } {
  return (
    result.success &&
    typeof result.before === 'string' &&
    typeof result.after === 'string'
  )
}

function OpSummaryTable({
  req,
  resp,
  running,
  preview,
}: {
  req: UpdateFileRequest
  resp: UpdateFileResponse | null
  running?: boolean
  preview?: boolean
}) {
  const totalApplied = resp?.results.reduce(
    (n, r) => n + (r.success ? r.applied : 0),
    0,
  )

  return (
    <table className="w-full font-mono text-[12px] text-ink">
      <thead>
        <tr className="border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          <th className="text-left font-normal px-3 py-1.5">path</th>
          <th className="text-left font-normal px-3 py-1.5">ops</th>
          <th className="text-left font-normal px-3 py-1.5 w-20">status</th>
        </tr>
      </thead>
      <tbody>
        {req.files.map((file, i) => {
          const result = resp?.results[i]
          const opSummary = file.ops
            .map((op) => {
              const head = formatUpdateOp(op)
              if (op.op === 'insert' || op.op === 'update_lines') {
                return `${head} · ${truncateInline(op.content)}`
              }
              return head
            })
            .join('; ')

          return (
            <tr
              key={file.path}
              className="border-b border-rule-2 last:border-b-0 align-top"
            >
              <td className="px-3 py-1.5 text-ink whitespace-nowrap">
                {file.path}
              </td>
              <td className="px-3 py-1.5 text-ink-faint break-all">
                {opSummary}
                {result?.success ? (
                  <span className="block text-[11px] text-ink-ghost mt-0.5 tabular-nums">
                    → {result.new_line_count} lines
                  </span>
                ) : null}
              </td>
              <td className="px-3 py-1.5">
                {preview || running ? (
                  <span className="text-ink-faint">—</span>
                ) : result?.success ? (
                  <span className="text-accent tabular-nums">
                    {result.applied} ok
                  </span>
                ) : result?.error ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="inline-flex items-center gap-1 text-warn cursor-help">
                        <TriangleAlert aria-hidden className="w-3.5 h-3.5" />
                        err
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>{result.error}</TooltipContent>
                  </Tooltip>
                ) : (
                  <span className="text-warn">err</span>
                )}
              </td>
            </tr>
          )
        })}
        {resp && typeof totalApplied === 'number' ? (
          <tr className="bg-paper-2">
            <td
              className="px-3 py-1.5 text-ink-faint uppercase tracking-[0.06em] text-[11px]"
              colSpan={2}
            >
              total applied
            </td>
            <td className="px-3 py-1.5">
              <FooterPill tone={totalApplied > 0 ? 'accent' : 'default'}>
                {`${totalApplied} ops`}
              </FooterPill>
            </td>
          </tr>
        ) : null}
      </tbody>
    </table>
  )
}

export function UpdateFileView({
  input,
  output,
  running,
  preview,
}: UpdateFileViewProps) {
  const req = safeParseRequest(updateFileRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(updateFileResponseSchema, output)
      : null

  const showDiffs =
    !preview &&
    !running &&
    resp?.results.some((r) => hasDiffSnapshot(r)) === true

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{req.files.length}</Chip>
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · applying…
          </span>
        ) : showDiffs ? (
          <Chip label="view">diff</Chip>
        ) : null}
      </div>

      {showDiffs ? (
        req.files.map((file, i) => {
          const result = resp?.results[i]
          if (result && !result.success) {
            return (
              <div
                key={file.path}
                className="border-b border-rule-2 px-3 py-2 flex items-center gap-2 font-mono text-[12px]"
              >
                <span className="text-ink">{file.path}</span>
                {result.error ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="inline-flex items-center gap-1 text-warn cursor-help">
                        <TriangleAlert aria-hidden className="w-3.5 h-3.5" />
                        err
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>{result.error}</TooltipContent>
                  </Tooltip>
                ) : (
                  <span className="text-warn">err</span>
                )}
              </div>
            )
          }

          if (result && hasDiffSnapshot(result)) {
            return (
              <div key={file.path} className="border-b border-rule-2">
                <div className="bg-paper-2 border-b border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
                  <span className="font-mono text-[12px] text-ink">
                    {file.path}
                  </span>
                  <Chip label="ops">{file.ops.length}</Chip>
                  <Chip label="applied">{result.applied}</Chip>
                  <Chip label="lines">{result.new_line_count}</Chip>
                </div>
                <CoderFileDiff
                  path={file.path}
                  oldContents={result.before}
                  newContents={result.after}
                />
              </div>
            )
          }

          return (
            <div
              key={file.path}
              className="border-b border-rule-2 px-3 py-2 font-mono text-[12px] text-ink-faint"
            >
              {file.path} · no diff snapshot
            </div>
          )
        })
      ) : (
        <OpSummaryTable
          req={req}
          resp={resp}
          running={running}
          preview={preview}
        />
      )}
    </div>
  )
}

export function UpdateFilePreview({ input }: { input: unknown }) {
  return <UpdateFileView input={input} preview />
}
