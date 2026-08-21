import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  ImageThumbnailButton,
  ImageViewer,
  useImageViewer,
} from '@/components/ui/ImageViewer'
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
          {req.full_page ? <Chip>Full page</Chip> : null}
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

  const images = shot.content.filter((b) => b.type === 'image' && b.data)
  const caption = shot.content.find((b) => b.type === 'text')?.text
  const mime = shot.mime || images[0]?.mime || 'image/png'
  const url = shot.url || req?.url || ''
  const sizeKb = Math.max(
    1,
    Math.round(
      (images.reduce((n, b) => n + (b.data?.length ?? 0), 0) * 3) / 4 / 1024,
    ),
  )
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="screenshot" variant="accent" />
        <Chip>{req?.fetcher ?? 'dynamic'}</Chip>
        <Chip>{mime.replace('image/', '')}</Chip>
        {req?.full_page ? <Chip>Full page</Chip> : null}
        {images.length > 1 ? <Chip>{images.length} tiles</Chip> : null}
        <Chip>
          <span className="tabular-nums">{sizeKb}</span>
          <span className="ml-0.5">KB</span>
        </Chip>
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{url}</span>
      </ActionLine>
      <div className="px-3 py-3 space-y-2">
        {images.map((b, i) => (
          <ScreenshotImage
            // biome-ignore lint/suspicious/noArrayIndexKey: tiles are positional
            key={i}
            src={`data:${b.mime || mime};base64,${b.data}`}
            alt={caption || `screenshot of ${url || 'page'}`}
            title={
              images.length > 1
                ? `Screenshot ${i + 1} of ${images.length}`
                : 'Screenshot'
            }
            description={url}
          />
        ))}
        {caption ? (
          <div className="font-mono text-[12.5px] text-ink-ghost break-all">
            {caption}
          </div>
        ) : null}
      </div>
    </div>
  )
}

function ScreenshotImage({
  src,
  alt,
  title,
  description,
}: {
  src: string
  alt: string
  title: string
  description?: string
}) {
  const viewer = useImageViewer()
  return (
    <>
      <ImageThumbnailButton
        title={title}
        onClick={viewer.show}
        className="block max-w-full"
      >
        <img
          src={src}
          alt={alt}
          loading="lazy"
          className="max-h-[420px] max-w-full border border-rule-2 bg-paper-2"
        />
      </ImageThumbnailButton>
      <ImageViewer
        open={viewer.open}
        onOpenChange={viewer.setOpen}
        src={src}
        alt={alt}
        title={title}
        description={description}
      />
    </>
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
        {req.full_page ? <Chip>Full page</Chip> : null}
      </MetaRow>
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{req.url}</span>
      </ActionLine>
    </div>
  )
}
