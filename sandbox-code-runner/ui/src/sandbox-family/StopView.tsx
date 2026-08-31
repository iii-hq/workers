/**
 * `sandbox::stop` — the warn slab. The id chip keeps its copy
 * affordance but drops the jump: a stopped sandbox has no fleet row
 * to open.
 */

import { safeParseResponse, stopRequestSchema, stopResponseSchema } from './parsers'
import { Chip, FooterPill, SandboxIdChip } from './shared'

interface StopViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function StopView({ input, output, running }: StopViewProps) {
  const req = stopRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData = output != null ? safeParseResponse(stopResponseSchema, output) : null

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-slab warn">
        <div className="cr-fam-line">
          <span className="cr-fam-warn">×</span>
          <span>{running ? 'stopping sandbox…' : 'stopped sandbox'}</span>
          <SandboxIdChip sandboxId={respData?.sandbox_id ?? req.data.sandbox_id} jump={false} />
        </div>
        <div className="cr-fam-chips">
          {req.data.wait ? <Chip label="wait">true</Chip> : null}
          {respData ? (
            <FooterPill tone={respData.stopped ? 'ok' : 'warn'}>
              {respData.stopped ? 'stopped' : 'not stopped'}
            </FooterPill>
          ) : null}
        </div>
      </div>
    </div>
  )
}
