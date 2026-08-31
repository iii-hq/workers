import { Chip, FilterChip, MetaRow, StatusPill } from '../../lib/shared'
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
          <StatusPill label="converting…" variant="default" />
          {chips}
        </MetaRow>
        <div className="br-ui-scrape-running">
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
        <StatusPill label={res.format} variant="accent" />
        <Chip>
          <span className="br-ui-scrape-num">
            {formatChars(res.content.length)}
          </span>
        </Chip>
        {truncated ? (
          <Chip className="br-ui-scrape-warning">
            <span>truncated</span>
          </Chip>
        ) : null}
      </MetaRow>
      {res.content.length === 0 ? (
        <div className="br-ui-scrape-empty">
          · empty
        </div>
      ) : (
        <pre className="br-ui-scrape-pre">
          <code>
            {res.content.slice(0, MAX_PREVIEW_CHARS)}
            {truncated ? '…' : ''}
          </code>
        </pre>
      )}
    </div>
  )
}
