import {
  formatBytes,
  formatMode,
  formatMtime,
  truncateMiddle,
} from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsReadRequestSchema,
  fsReadResponseSchema,
  safeParseResponse,
} from './parsers'
import { TargetChip } from './shared'

interface FsReadViewProps {
  input: unknown
  output: unknown
}

/** shell::fs::read never inlines content — the response `content` is
    always a channel ref, so the body is the stream row promoted to the
    only branch (no CodeHighlight, no empty state). The console cannot
    dereference the channel, so there is no "view content" affordance. */
export function FsReadView({ input, output }: FsReadViewProps) {
  const req = fsReadRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsReadResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          file
        </span>
        <code className="font-mono text-[12px] text-ink">{req.data.path}</code>
        <TargetChip target={req.data.target} />
      </div>

      <div className="px-3 py-3 font-mono text-[12.5px] text-ink-faint flex flex-wrap items-center gap-1.5">
        <span>content streamed via channel</span>
        <code className="bg-paper-2 border border-rule-2 px-1.5 py-0.5 text-ink">
          {truncateMiddle(resp.content.channel_id, 18)}
        </code>
        <span className="text-ink-ghost">
          ({resp.content.direction ?? 'read'})
        </span>
      </div>

      <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
        <Chip label="size">{formatBytes(resp.size)}</Chip>
        <Chip label="mode">{formatMode(resp.mode)}</Chip>
        <Chip label="mtime">{formatMtime(resp.mtime)}</Chip>
        <FooterPill tone="default">streamed</FooterPill>
      </div>
    </div>
  )
}
