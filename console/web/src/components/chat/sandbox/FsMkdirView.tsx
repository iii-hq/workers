import {
  fsMkdirRequestSchema,
  fsMkdirResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip } from './terminal/Terminal'

interface FsMkdirViewProps {
  input: unknown
  output: unknown
}

export function FsMkdirView({ input, output }: FsMkdirViewProps) {
  const req = fsMkdirRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsMkdirResponseSchema, output)
  if (!resp) return null
  const created = resp.created

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className={created ? 'text-accent' : 'text-ink-faint'}>
            {created ? '+ created ' : '· exists '}
          </span>
          <span>{req.data.path}</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip label="mode">{req.data.mode ?? '0755'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
        </div>
      </div>
    </div>
  )
}
