import { Badge, Chip, EmptyState, MetaRow } from '@iii-dev/console-ui'
import { FilterChip } from '../../lib/shared'
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
  if (!req) return null
  const chips = (
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
  )

  if (running) {
    return (
      <div className="br-ui-scrape-section">
        <MetaRow>
          <Badge variant="default">converting…</Badge>
          {chips}
        </MetaRow>
        <div className="br-ui-more">
          · converting…
        </div>
      </div>
    )
  }

  const res = safeParseResponse(markdownResponseSchema, output)
  if (!res) return null
  const truncated = res.content.length > MAX_PREVIEW_CHARS
  return (
    <div className="br-ui-scrape-section">
      <MetaRow>
        <Badge variant="accent">{res.format}</Badge>
        <Chip>
          <span className="br-ui-num">{formatChars(res.content.length)}</span>
        </Chip>
        {truncated ? (
          <Chip tone="warning">
            <span>truncated</span>
          </Chip>
        ) : null}
      </MetaRow>
      {res.content.length === 0 ? (
        <EmptyState title="Empty" description="The conversion produced no text." />
      ) : (
        <pre className="br-ui-text">
          <code>
            {res.content.slice(0, MAX_PREVIEW_CHARS)}
            {truncated ? '…' : ''}
          </code>
        </pre>
      )}
    </div>
  )
}
