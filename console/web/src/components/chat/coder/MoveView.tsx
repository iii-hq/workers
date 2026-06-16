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
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
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

      <table className="w-full font-mono text-[12px] text-ink">
        <thead>
          <tr className="border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            <th className="text-left font-normal px-3 py-1.5">from → to</th>
            <th className="text-left font-normal px-3 py-1.5 w-28">outcome</th>
            <th className="text-left font-normal px-3 py-1.5 w-16">status</th>
          </tr>
        </thead>
        <tbody>
          {req.files.map((spec, i) => {
            const result = resp?.results[i]
            // Canonical absolute paths once the wire responds (caller's
            // input verbatim when resolution failed); request paths until.
            const from = result?.from ?? spec.from
            const to = result?.to ?? spec.to
            const outcome = pending
              ? { label: 'pending', tone: 'text-ink-faint' }
              : !result
                ? { label: '—', tone: 'text-ink-faint' }
                : result.moved
                  ? { label: 'moved', tone: 'text-ink' }
                  : result.success
                    ? { label: 'unchanged', tone: 'text-ink-ghost' }
                    : { label: 'failed', tone: 'text-ink-ghost' }

            return (
              <tr
                key={`${spec.from}→${spec.to}`}
                className="border-b border-rule-2 last:border-b-0 align-top"
              >
                <td className="px-3 py-1.5">
                  <div className="flex flex-wrap items-baseline gap-2">
                    <span>{from}</span>
                    <span className="text-ink-ghost">→</span>
                    <span>{to}</span>
                    {spec.overwrite ? (
                      <Chip label="overwrite" className="border-warn text-warn">
                        true
                      </Chip>
                    ) : null}
                  </div>
                  {result?.error ? (
                    <div className="mt-1 text-[11px] text-warn break-words">
                      {result.error.message}
                    </div>
                  ) : null}
                </td>
                <td className="px-3 py-1.5">
                  <span className={outcome.tone}>{outcome.label}</span>
                </td>
                <td className="px-3 py-1.5">
                  {pending || !result ? (
                    <span className="text-ink-faint">—</span>
                  ) : result.success ? (
                    <span className="text-accent">ok</span>
                  ) : result.error ? (
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
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

export function MovePreview({ input }: { input: unknown }) {
  return <MoveView input={input} preview />
}
