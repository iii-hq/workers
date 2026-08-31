/** `sandbox::fs::mkdir` — created vs already-exists. */

import { fsMkdirRequestSchema, fsMkdirResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

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
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <span className={created ? 'cr-fam-accent' : 'faint'}>{created ? '+ created ' : '· exists '}</span>
          <span>{req.data.path}</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          <Chip label="mode">{req.data.mode ?? '0755'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
        </div>
      </div>
    </div>
  )
}
