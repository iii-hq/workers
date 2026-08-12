/** `sandbox::fs::mv` — src → dst. */

import { fsMvRequestSchema, fsMvResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

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
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <span className={moved ? 'cr-fam-accent' : 'faint'}>{moved ? 'mv' : '·'}</span>
          <span>{req.data.src}</span>
          <span className="ghost">→</span>
          <span>{req.data.dst}</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          {req.data.overwrite ? <Chip label="overwrite">true</Chip> : null}
        </div>
      </div>
    </div>
  )
}
