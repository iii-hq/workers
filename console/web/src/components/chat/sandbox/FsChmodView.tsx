import { formatMode } from './format'
import {
  fsChmodRequestSchema,
  fsChmodResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip } from './terminal/Terminal'

interface FsChmodViewProps {
  input: unknown
  output: unknown
}

export function FsChmodView({ input, output }: FsChmodViewProps) {
  const req = fsChmodRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsChmodResponseSchema, output)
  if (!resp) return null
  const ownership =
    typeof req.data.uid === 'number' || typeof req.data.gid === 'number'
      ? `${req.data.uid ?? '_'}:${req.data.gid ?? '_'}`
      : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] flex flex-wrap items-baseline gap-2 text-ink">
          <span className="text-ink-faint">chmod</span>
          <span>{req.data.path}</span>
          <span className="text-ink-ghost">→</span>
          <span className="tabular-nums">{req.data.mode}</span>
          <span className="text-ink-faint">({formatMode(req.data.mode)})</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {ownership ? <Chip label="own">{ownership}</Chip> : null}
          {req.data.recursive ? <Chip label="recursive">true</Chip> : null}
          <Chip label="updated">{resp.updated}</Chip>
        </div>
      </div>
    </div>
  )
}
