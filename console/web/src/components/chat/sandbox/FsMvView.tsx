import {
  fsMvRequestSchema,
  fsMvResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip } from './terminal/Terminal'

interface FsMvViewProps {
  input: unknown
  output: unknown
}

export function FsMvView({ input, output }: FsMvViewProps) {
  const req = fsMvRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsMvResponseSchema, output)
  if (!resp) return null
  const moved = resp.moved

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] flex flex-wrap items-baseline gap-2 text-ink">
          <span className={moved ? 'text-accent' : 'text-ink-faint'}>
            {moved ? 'mv' : '·'}
          </span>
          <span>{req.data.src}</span>
          <span className="text-ink-ghost">→</span>
          <span>{req.data.dst}</span>
        </div>
        {req.data.overwrite ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <Chip label="overwrite">true</Chip>
          </div>
        ) : null}
      </div>
    </div>
  )
}
