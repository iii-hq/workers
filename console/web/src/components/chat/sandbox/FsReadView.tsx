import { CodeHighlight } from '@/lib/syntax'
import {
  formatBytes,
  formatMode,
  formatMtime,
  inferLangFromPath,
  truncateMiddle,
} from './format'
import {
  fsReadRequestSchema,
  fsReadResponseSchema,
  safeParseResponse,
  streamChannelRefSchema,
} from './parsers'
import { Chip } from './terminal/Terminal'

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
  const stream =
    inline === null ? streamChannelRefSchema.safeParse(resp.content) : null
  const lang = inferLangFromPath(req.data.path)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          file
        </span>
        <code className="font-mono text-[12px] text-ink">{req.data.path}</code>
      </div>

      {inline !== null ? (
        lang ? (
          <CodeHighlight code={inline} language={lang} wrap />
        ) : (
          <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
            <code>{inline}</code>
          </pre>
        )
      ) : stream?.success ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-faint flex flex-wrap items-center gap-1.5">
          <span>streaming via channel</span>
          <code className="bg-paper-2 border border-rule-2 px-1.5 py-0.5 text-ink">
            {truncateMiddle(stream.data.channel_id, 18)}
          </code>
          <span className="text-ink-ghost">({stream.data.direction})</span>
        </div>
      ) : (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · empty
        </div>
      )}

      <div className="bg-paper-2 border-t border-rule-2 px-3 py-1.5 flex flex-wrap items-center gap-1.5">
        <Chip label="size">{formatBytes(resp.size)}</Chip>
        <Chip label="mode">{formatMode(resp.mode)}</Chip>
        <Chip label="mtime">{formatMtime(resp.mtime)}</Chip>
      </div>
    </div>
  )
}
