/**
 * `coder::delete-file` — path-level removal summary (no file body on wire).
 */
import { TriangleAlert } from 'lucide-react'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
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

      <table className="w-full font-mono text-[12px] text-ink">
        <thead>
          <tr className="border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            <th className="text-left font-normal px-3 py-1.5">path</th>
            <th className="text-left font-normal px-3 py-1.5 w-28">outcome</th>
            <th className="text-left font-normal px-3 py-1.5 w-16">status</th>
          </tr>
        </thead>
        <tbody>
          {req.paths.map((path, i) => {
            const result = resp?.results[i]
            const removed = result?.removed
            const outcomeLabel =
              preview || running
                ? 'pending'
                : removed
                  ? 'removed'
                  : 'not removed'

            return (
              <tr key={path} className="border-b border-rule-2 last:border-b-0">
                <td className="px-3 py-1.5">{path}</td>
                <td className="px-3 py-1.5">
                  <span
                    className={
                      removed
                        ? 'text-warn'
                        : preview || running
                          ? 'text-ink-faint'
                          : 'text-ink-ghost'
                    }
                  >
                    {outcomeLabel}
                  </span>
                </td>
                <td className="px-3 py-1.5">
                  {preview || running ? (
                    <span className="text-ink-faint">—</span>
                  ) : result?.success ? (
                    <span className="text-accent">ok</span>
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
        </tbody>
      </table>
    </div>
  )
}

export function DeleteFilePreview({ input }: { input: unknown }) {
  return <DeleteFileView input={input} preview />
}
