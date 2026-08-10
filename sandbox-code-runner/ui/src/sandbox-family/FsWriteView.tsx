/** `sandbox::fs::write` — bytes written + how the content travelled. */

import { formatBytes } from './format'
import { fsWriteRequestSchema, fsWriteResponseSchema, safeParseResponse, streamChannelRefSchema } from './parsers'
import { Chip, FooterPill, SandboxIdChip } from './shared'

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
  const usedB64 = !!req.data.content_b64

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-slab">
        <div className="cr-fam-line">
          <span className="cr-fam-accent">+ wrote</span>
          <span className="num">{formatBytes(resp.bytes_written)}</span>
          <span className="faint">to</span>
          <span>{resp.path}</span>
        </div>
        <div className="cr-fam-chips">
          <SandboxIdChip sandboxId={req.data.sandbox_id} />
          <Chip label="mode">{req.data.mode ?? '0644'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
          {streamed ? <FooterPill tone="default">uploaded via channel</FooterPill> : null}
          {usedB64 ? <FooterPill tone="default">base64 inline</FooterPill> : null}
        </div>
      </div>
    </div>
  )
}
