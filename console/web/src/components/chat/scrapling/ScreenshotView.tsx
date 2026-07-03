import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
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

  if (running) {
    if (!req) return null
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="capturing…" variant="default" />
          <Chip>{req.fetcher ?? 'dynamic'}</Chip>
          {req.full_page ? <Chip>full page</Chip> : null}
        </MetaRow>
        <ActionLine symbol="→" tone="ink">
          <span className="break-all">{req.url}</span>
        </ActionLine>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · waiting for the browser…
        </div>
      </div>
    )
  }

  const shot = safeParseResponse(screenshotResponseSchema, output)
  if (!shot) return null

  const mime = shot.mime || 'image/png'
  const url = shot.url || req?.url || ''
  const sizeKb = Math.max(
    1,
    Math.round((shot.image_base64.length * 3) / 4 / 1024),
  )
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="screenshot" variant="accent" />
        <Chip>{req?.fetcher ?? 'dynamic'}</Chip>
        <Chip>{mime.replace('image/', '')}</Chip>
        {req?.full_page ? <Chip>full page</Chip> : null}
        <Chip>
          <span className="tabular-nums">{sizeKb}</span>
          <span className="ml-0.5">KB</span>
        </Chip>
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{url}</span>
      </ActionLine>
      <div className="px-3 py-3">
        <img
          src={`data:${mime};base64,${shot.image_base64}`}
          alt={`screenshot of ${url || 'page'}`}
          loading="lazy"
          className="max-w-full max-h-[420px] border border-rule-2 bg-paper-2"
        />
      </div>
    </div>
  )
}

export function ScreenshotPreview({ input }: { input: unknown }) {
  const req = safeParseRequest(screenshotRequestSchema, input)
  if (!req) return null
  return (
    <div className="border-b border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="permission to screenshot" variant="warn" />
        <Chip>{req.fetcher ?? 'dynamic'}</Chip>
        {req.format ? <Chip>{req.format}</Chip> : null}
        {req.full_page ? <Chip>full page</Chip> : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{req.url}</span>
      </ActionLine>
    </div>
  )
}
