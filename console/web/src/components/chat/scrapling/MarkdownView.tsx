import { FilterChip } from '@/components/chat/engine/shared'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  formatChars,
  markdownRequestSchema,
  markdownResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

const MAX_PREVIEW_CHARS = 4000

export function MarkdownView({
  input,
  output,
  running,
}: {
  input: unknown
  output: unknown
  running?: boolean
}) {
  const req = safeParseRequest(markdownRequestSchema, input)
  const chips = req ? (
    <>
      <Chip>{req.format ?? 'markdown'}</Chip>
      {req.css_selector ? (
        <FilterChip label="scope" value={req.css_selector} />
      ) : null}
      {req.main_content_only ? <Chip>main only</Chip> : null}
      {req.html != null ? (
        <FilterChip label="html in" value={formatChars(req.html.length)} />
      ) : null}
    </>
  ) : null

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="converting…" variant="default" />
          {chips}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · converting…
        </div>
      </div>
    )
  }

  const res = safeParseResponse(markdownResponseSchema, output)
  if (!res) return null
  const truncated = res.content.length > MAX_PREVIEW_CHARS
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={res.format} variant="accent" />
        <Chip>
          <span className="tabular-nums">
            {formatChars(res.content.length)}
          </span>
        </Chip>
        {truncated ? (
          <Chip className="text-warn border-warn/40">
            <span className="uppercase tracking-[0.06em]">truncated</span>
          </Chip>
        ) : null}
      </MetaRow>
      {res.content.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · empty
        </div>
      ) : (
        <pre className="px-3 py-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-words m-0">
          <code>
            {res.content.slice(0, MAX_PREVIEW_CHARS)}
            {truncated ? '…' : ''}
          </code>
        </pre>
      )}
    </div>
  )
}
