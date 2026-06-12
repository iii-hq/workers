import { SedResultsTable } from '@/components/chat/sandbox/FsSedView'
import { truncateMiddle } from '@/components/chat/sandbox/format'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsSedRequestSchema,
  fsSedResponseSchema,
  safeParseResponse,
} from './parsers'
import { TargetChip } from './shared'

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
  const pathMode = req.data.path != null
  const target =
    req.data.path ??
    (req.data.files?.length ? `${req.data.files.length} files` : '—')

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
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
      </div>

      {results.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · no files touched
        </div>
      ) : (
        <SedResultsTable
          results={results}
          totalReplacements={total_replacements}
        />
      )}
    </div>
  )
}
