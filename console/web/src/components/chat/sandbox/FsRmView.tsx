import {
  fsRmRequestSchema,
  fsRmResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip } from './terminal/Terminal'

interface FsRmViewProps {
  input: unknown
  output: unknown
}

export function FsRmView({ input, output }: FsRmViewProps) {
  const req = fsRmRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsRmResponseSchema, output)
  if (!resp) return null
  const removed = resp.removed

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className={removed ? 'text-warn' : 'text-ink-faint'}>
            {removed ? '− removed ' : '· not removed '}
          </span>
          <span>{req.data.path}</span>
        </div>
        {req.data.recursive ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <Chip label="recursive" className="border-warn text-warn">
              true
            </Chip>
          </div>
        ) : null}
      </div>
    </div>
  )
}
