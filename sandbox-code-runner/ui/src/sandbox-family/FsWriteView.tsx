/** `sandbox::fs::write` — bytes written + how the content travelled. */

import { Badge } from '@iii-dev/console-ui'
import { Plus } from 'lucide-react'
import { formatBytes } from './format'
import { fsWriteRequestSchema, fsWriteResponseSchema, safeParseResponse, streamChannelRefSchema } from './parsers'
import { Chip, SandboxIdChip } from './shared'

interface FsWriteViewProps {
  input: unknown
  output: unknown
}

export function FsWriteView({ input, output }: FsWriteViewProps) {
  const req = fsWriteRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsWriteResponseSchema, output)
  if (!resp) return null
  const streamed = req.data.content ? streamChannelRefSchema.safeParse(req.data.content).success : false
  // Null-check, not truthiness: writing an empty file via content_b64: ""
  // still travelled as base64.
  const usedB64 = req.data.content_b64 != null

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <Plus size={16} aria-hidden className="cr-fam-accent" />
          <span className="cr-fam-accent">wrote</span>
          <span className="num">{formatBytes(resp.bytes_written)}</span>
          <span className="faint">to</span>
          <span>{resp.path}</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          <Chip label="mode">{req.data.mode ?? '0644'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
          {streamed ? <Badge>uploaded via channel</Badge> : null}
          {usedB64 ? <Badge>base64 inline</Badge> : null}
        </div>
      </div>
    </div>
  )
}
