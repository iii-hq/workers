/** `sandbox::fs::rm` — removed vs not-removed, recursive flagged loud. */

import { fsRmRequestSchema, fsRmResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

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
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <span className={removed ? 'cr-fam-warn' : 'faint'}>{removed ? '− removed ' : '· not removed '}</span>
          <span>{req.data.path}</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          {req.data.recursive ? (
            <Chip label="recursive" className="warn">
              true
            </Chip>
          ) : null}
        </div>
      </div>
    </div>
  )
}
