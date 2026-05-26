import { truncateMiddle } from './format'
import {
  safeParseResponse,
  stopRequestSchema,
  stopResponseSchema,
} from './parsers'
import { Chip, FooterPill } from './terminal/Terminal'

interface StopViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function StopView({ input, output, running }: StopViewProps) {
  const req = stopRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(stopResponseSchema, output) : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-warn">
        <div className="font-mono text-[12.5px] text-ink flex flex-wrap items-baseline gap-2">
          <span className="text-warn">×</span>
          <span>{running ? 'stopping sandbox…' : 'stopped sandbox'}</span>
          <code className="bg-paper-2 border border-rule-2 px-1.5 py-0.5 text-[12px] text-ink">
            {truncateMiddle(respData?.sandbox_id ?? req.data.sandbox_id, 24)}
          </code>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {req.data.wait ? <Chip label="wait">true</Chip> : null}
          {respData ? (
            <FooterPill tone={respData.stopped ? 'accent' : 'warn'}>
              {respData.stopped ? 'stopped' : 'not stopped'}
            </FooterPill>
          ) : null}
        </div>
      </div>
    </div>
  )
}
