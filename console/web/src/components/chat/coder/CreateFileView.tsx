/**
 * `coder::create-file` — input-derived diffs via @pierre/diffs.
 *
 * UI-only: overwrite shows new content only (previous version unknown on
 * wire). Entries are independent — one failure never aborts the batch, so
 * each row gets its own success/error state. Per-entry errors are
 * structured WireError {code, message}; the message names the corrective
 * next call (e.g. C217 → retry with overwrite: true) and is shown verbatim.
 */
import { TriangleAlert } from 'lucide-react'
import { formatBytes } from '@/components/chat/sandbox/format'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { CoderNewFilePreview, CoderOverwritePreview } from './CoderDiff'
import { OpenInEditorButton } from './OpenInEditorButton'
import {
  createFileRequestSchema,
  createFileResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface CreateFileViewProps {
  input: unknown
  output?: unknown
  running?: boolean
  /** When true, skip filtering by response success (approval preview). */
  preview?: boolean
}

export function CreateFileView({
  input,
  output,
  running,
  preview,
}: CreateFileViewProps) {
  const req = safeParseRequest(createFileRequestSchema, input)
  if (!req) return null
  const resp =
    output != null && !preview
      ? safeParseResponse(createFileResponseSchema, output)
      : null

  const parentsAny = req.files.some((f) => f.parents !== false)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{req.files.length}</Chip>
        {parentsAny ? <Chip label="parents">true</Chip> : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · writing…
          </span>
        ) : null}
      </div>

      {req.files.map((file, i) => {
        const result = resp?.results[i]
        if (result && !result.success) {
          // Result path is canonical absolute (jail-resolved) unless
          // resolution itself failed — then it's the caller's input
          // verbatim, never absolute. Flag that so it reads as raw input.
          const unresolved = !result.path.startsWith('/')
          return (
            <div
              key={file.path}
              className="border-b border-rule-2 last:border-b-0 px-3 py-2 flex flex-wrap items-center gap-2 font-mono text-[12px]"
            >
              <span className="text-ink">{result.path}</span>
              {unresolved ? (
                <span className="text-ink-ghost text-[11px]">· unresolved</span>
              ) : null}
              {result.error ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="inline-flex items-center gap-1 text-warn cursor-help">
                      <TriangleAlert aria-hidden className="w-3.5 h-3.5" />
                      {result.error.code}
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{result.error.message}</TooltipContent>
                </Tooltip>
              ) : (
                <span className="text-warn">err</span>
              )}
            </div>
          )
        }

        if (result?.success) {
          return (
            <div key={file.path}>
              <div className="bg-paper-2 border-b border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
                <Chip label="wrote">{formatBytes(result.bytes_written)}</Chip>
                <span className="font-mono text-[12px] text-ink">
                  {file.path}
                </span>
                <OpenInEditorButton path={result.path} />
              </div>
              {file.overwrite ? (
                <CoderOverwritePreview
                  path={file.path}
                  content={file.content}
                />
              ) : (
                <CoderNewFilePreview path={file.path} content={file.content} />
              )}
            </div>
          )
        }

        // Pending preview or running — show intended change from input.
        if (file.overwrite) {
          return (
            <CoderOverwritePreview
              key={file.path}
              path={file.path}
              content={file.content}
            />
          )
        }
        return (
          <CoderNewFilePreview
            key={file.path}
            path={file.path}
            content={file.content}
          />
        )
      })}
    </div>
  )
}

export function CreateFilePreview({ input }: { input: unknown }) {
  return <CreateFileView input={input} preview />
}
