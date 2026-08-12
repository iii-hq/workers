/** `sandbox::fs::stat` — one entry's metadata as chips. */

import { formatBytes, formatMode, formatMtime } from './format'
import { fsStatRequestSchema, fsStatResponseSchema, safeParseResponse } from './parsers'
import { Chip, FooterPill, SandboxIdChip } from './shared'

interface FsStatViewProps {
  input: unknown
  output: unknown
}

export function FsStatView({ input, output }: FsStatViewProps) {
  const req = fsStatRequestSchema.safeParse(input)
  if (!req.success) return null
  const e = safeParseResponse(fsStatResponseSchema, output)
  if (!e) return null

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <span className="faint">stat </span>
          <span>{req.data.path}</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          <Chip label="size">{e.is_dir ? '—' : formatBytes(e.size)}</Chip>
          <Chip label="mode">{`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}</Chip>
          <Chip label="mtime">{formatMtime(e.mtime)}</Chip>
          {e.is_dir ? <FooterPill tone="default">dir</FooterPill> : null}
          {e.is_symlink ? <FooterPill tone="warn">symlink</FooterPill> : null}
        </div>
      </div>
    </div>
  )
}
