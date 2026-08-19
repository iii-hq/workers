import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '../../lib/shared'
import {
  safeParseRequest,
  safeParseResponse,
  screenshotRequestSchema,
  screenshotResponseSchema,
} from './parsers'

interface ScreenshotViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function ScreenshotView({
  input,
  output,
  running,
}: ScreenshotViewProps) {
  const req = safeParseRequest(screenshotRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <div className="br-ui-scrape-section">
        <MetaRow>
          <StatusPill label="capturing…" variant="default" />
          <Chip>{req.fetcher ?? 'dynamic'}</Chip>
          {req.full_page ? <Chip>full page</Chip> : null}
          {req.proxy ? <Chip>proxy</Chip> : null}
        </MetaRow>
        <ActionLine symbol="→" tone="ink">
          <span className="br-ui-scrape-break">{req.url}</span>
        </ActionLine>
        <div className="br-ui-scrape-running">
          · waiting for the browser…
        </div>
      </div>
    )
  }

  const shot = safeParseResponse(screenshotResponseSchema, output)
  if (!shot) return null

  const images = shot.content.filter((b) => b.type === 'image' && b.data)
  const caption = shot.content.find((b) => b.type === 'text')?.text
  const mime = shot.mime || images[0]?.mime || 'image/png'
  const url = shot.url || req.url
  const sizeKb = Math.max(
    1,
    Math.round(
      (images.reduce((n, b) => n + (b.data?.length ?? 0), 0) * 3) / 4 / 1024,
    ),
  )
  return (
    <div className="br-ui-scrape-section">
      <MetaRow>
        <StatusPill label="screenshot" variant="accent" />
        <Chip>{req.fetcher ?? 'dynamic'}</Chip>
        <Chip>{mime.replace('image/', '')}</Chip>
        {req.full_page ? <Chip>full page</Chip> : null}
        {req.proxy ? <Chip>proxy</Chip> : null}
        {images.length > 1 ? <Chip>{images.length} tiles</Chip> : null}
        <Chip>
          <span className="br-ui-scrape-num">{sizeKb}</span>
          <span className="br-ui-scrape-unit">KB</span>
        </Chip>
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="br-ui-scrape-break">{url}</span>
      </ActionLine>
      <div className="br-ui-scrape-gallery">
        {images.map((b, i) => (
          <img
            key={i}
            src={`data:${b.mime || mime};base64,${b.data}`}
            alt={caption || `screenshot of ${url || 'page'}`}
            loading="lazy"
            className="br-ui-scrape-image"
          />
        ))}
        {caption ? (
          <div className="br-ui-scrape-caption">
            {caption}
          </div>
        ) : null}
      </div>
    </div>
  )
}

export function ScreenshotPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(screenshotRequestSchema, input)
  if (!req) return null
  return (
    <div className="br-ui-scrape-section is-preview">
      <MetaRow>
        <StatusPill label="permission to screenshot" variant="warn" />
        <Chip>{req.fetcher ?? 'dynamic'}</Chip>
        {req.format ? <Chip>{req.format}</Chip> : null}
        {req.full_page ? <Chip>full page</Chip> : null}
        {req.proxy ? <Chip>proxy</Chip> : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="br-ui-scrape-break">{req.url}</span>
      </ActionLine>
    </div>
  )
}
