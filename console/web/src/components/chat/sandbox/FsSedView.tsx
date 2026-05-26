import { TriangleAlert } from 'lucide-react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import {
  fsSedRequestSchema,
  fsSedResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip, FooterPill } from './terminal/Terminal'

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
  const target =
    req.data.path ??
    (req.data.files?.length ? `${req.data.files.length} files` : '—')

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="target">{target}</Chip>
        <Chip label="pattern">{req.data.pattern}</Chip>
        <Chip label="→">{req.data.replacement || "''"}</Chip>
        {req.data.regex === false ? <Chip>literal</Chip> : null}
        {req.data.first_only ? <Chip>first-only</Chip> : null}
        {req.data.ignore_case ? <Chip>case-insensitive</Chip> : null}
      </div>

      {results.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no files touched
        </div>
      ) : (
        <table className="w-full font-mono text-[12px] text-ink">
          <thead>
            <tr className="border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              <th className="text-left font-normal px-3 py-1.5">path</th>
              <th className="text-left font-normal px-2 py-1.5 w-20 tabular-nums">
                replacements
              </th>
              <th className="text-left font-normal px-3 py-1.5 w-8">status</th>
            </tr>
          </thead>
          <tbody>
            {results.map((r) => (
              <tr
                key={r.path}
                className="border-b border-rule-2 last:border-b-0"
              >
                <td className="px-3 py-1.5 text-ink">{r.path}</td>
                <td className="px-2 py-1.5 text-ink-faint tabular-nums">
                  {r.replacements}
                </td>
                <td className="px-3 py-1.5">
                  {r.success ? (
                    <span className="text-accent">ok</span>
                  ) : r.error ? (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="inline-flex items-center gap-1 text-warn cursor-help">
                          <TriangleAlert aria-hidden className="w-3.5 h-3.5" />
                          err
                        </span>
                      </TooltipTrigger>
                      <TooltipContent>{r.error}</TooltipContent>
                    </Tooltip>
                  ) : (
                    <span className="text-warn">err</span>
                  )}
                </td>
              </tr>
            ))}
            <tr className="bg-paper-2">
              <td className="px-3 py-1.5 text-ink-faint uppercase tracking-[0.06em] text-[11px]">
                total
              </td>
              <td className="px-2 py-1.5 tabular-nums" colSpan={2}>
                <FooterPill
                  tone={total_replacements > 0 ? 'accent' : 'default'}
                >
                  {`${total_replacements} replacements`}
                </FooterPill>
              </td>
            </tr>
          </tbody>
        </table>
      )}
    </div>
  )
}
