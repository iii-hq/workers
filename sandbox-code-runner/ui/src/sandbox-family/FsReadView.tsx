/**
 * `sandbox::fs::read` — file body with path-inferred highlighting, or
 * the StreamChannelRef card when the daemon streamed the content
 * instead of inlining it (≥ 1 MiB — `INLINE_BUFFER_CAP` in fs/read.rs).
 */

import { CodeHighlight } from '@iii-dev/console-ui'
import { formatBytes, formatMode, formatMtime, inferLangFromPath, truncateMiddle } from './format'
import { fsReadRequestSchema, fsReadResponseSchema, safeParseResponse, streamChannelRefSchema } from './parsers'
import { Chip, SandboxIdChip } from './shared'

interface FsReadViewProps {
  input: unknown
  output: unknown
}

export function FsReadView({ input, output }: FsReadViewProps) {
  const req = fsReadRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsReadResponseSchema, output)
  if (!resp) return null
  const inline = typeof resp.content === 'string' ? resp.content : null
  const stream = inline === null ? streamChannelRefSchema.safeParse(resp.content) : null
  const lang = inferLangFromPath(req.data.path)

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-chips-row">
        <SandboxIdChip sandboxId={req.data.sandbox_id} />
        <span className="cr-fam-file-k">file</span>
        <code className="cr-fam-file">{req.data.path}</code>
      </div>

      {inline !== null ? (
        <div className="cr-fam-code">
          <CodeHighlight code={inline} language={lang ?? 'text'} wrap />
        </div>
      ) : stream?.success ? (
        <div className="cr-fam-note cr-fam-stream-ref">
          <span>streaming via channel</span>
          <code className="cr-fam-code-chip">{truncateMiddle(stream.data.channel_id, 18)}</code>
          <span className="ghost">({stream.data.direction})</span>
        </div>
      ) : (
        <div className="cr-fam-note-ghost">· empty</div>
      )}

      <div className="cr-fam-foot">
        <Chip label="size">{formatBytes(resp.size)}</Chip>
        <Chip label="mode">{formatMode(resp.mode)}</Chip>
        <Chip label="mtime">{formatMtime(resp.mtime)}</Chip>
      </div>
    </div>
  )
}
