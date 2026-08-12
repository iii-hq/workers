/** `sandbox::fs::chmod` — mode change with the decoded rwx string. */

import { formatMode } from './format'
import { fsChmodRequestSchema, fsChmodResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

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
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <span className="faint">chmod</span>
          <span>{req.data.path}</span>
          <span className="ghost">→</span>
          <span className="num">{req.data.mode}</span>
          <span className="faint">({formatMode(req.data.mode)})</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          {ownership ? <Chip label="own">{ownership}</Chip> : null}
          {req.data.recursive ? <Chip label="recursive">true</Chip> : null}
          <Chip label="updated">{resp.updated}</Chip>
        </div>
      </div>
    </div>
  )
}
