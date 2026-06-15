import {
  formatBytes,
  formatMode,
  formatMtime,
} from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  fsStatRequestSchema,
  fsStatResponseSchema,
  safeParseResponse,
} from './parsers'
import { TargetChip } from './shared'

interface FsStatViewProps {
  input: unknown
  output: unknown
}

/** Response is the bare FsEntry (`StatResponse` is serde-transparent). */
export function FsStatView({ input, output }: FsStatViewProps) {
  const req = fsStatRequestSchema.safeParse(input)
  if (!req.success) return null
  const e = safeParseResponse(fsStatResponseSchema, output)
  if (!e) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className="text-ink-faint">stat </span>
          <span>{req.data.path}</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip label="size">{e.is_dir ? '—' : formatBytes(e.size)}</Chip>
          <Chip label="mode">{`${e.is_dir ? 'd' : '-'}${formatMode(e.mode)}`}</Chip>
          <Chip label="mtime">{formatMtime(e.mtime)}</Chip>
          <TargetChip target={req.data.target} />
          {e.is_dir ? <FooterPill tone="default">dir</FooterPill> : null}
          {e.is_symlink ? <FooterPill tone="warn">symlink</FooterPill> : null}
        </div>
      </div>
    </div>
  )
}
