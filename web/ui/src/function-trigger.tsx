import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
} from '@iii-dev/console-ui'

interface ViewableImage {
  dataUrl: string
  mime: string
  label: string
  url?: string
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value != null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function imageFrom(value: unknown, fallbackUrl?: string): ViewableImage | null {
  const queue: unknown[] = [value]
  const seen = new Set<object>()
  for (let depth = 0; queue.length > 0 && depth < 12; depth += 1) {
    const current = queue.shift()
    const record = asRecord(current)
    if (!record || seen.has(record)) continue
    seen.add(record)

    if (Array.isArray(record.content)) {
      const image = record.content.find((item) => {
        const block = asRecord(item)
        return block?.type === 'image' && typeof block.data === 'string'
      })
      const block = asRecord(image)
      if (block && typeof block.data === 'string') {
        const mime =
          typeof block.mime === 'string' ? block.mime : 'application/octet-stream'
        if (!mime.startsWith('image/')) return null
        return {
          dataUrl: `data:${mime};base64,${block.data}`,
          mime,
          label: `Fetched ${mime.replace('image/', '').toUpperCase()} image`,
          url: fallbackUrl,
        }
      }
      queue.push(...record.content)
    }
    if ('details' in record) queue.push(record.details)
  }
  return null
}

function requestUrl(input: unknown): string | undefined {
  const direct = asRecord(input)
  if (typeof direct?.url === 'string') return direct.url
  const details = asRecord(direct?.details)
  return typeof details?.url === 'string' ? details.url : undefined
}

function renderImage(message: FunctionTriggerMessage): React.ReactNode | null {
  if (
    message.functionId !== 'web::fetch' ||
    message.pendingApproval ||
    message.running ||
    message.output == null
  ) {
    return null
  }
  const image = imageFrom(message.output, requestUrl(message.input))
  if (!image) return null
  return (
    <figure className="web-ui-image">
      <img
        className="web-ui-image__asset"
        src={image.dataUrl}
        alt={image.url ? `Image fetched from ${image.url}` : image.label}
      />
      <figcaption className="web-ui-image__caption">
        <span>{image.label}</span>
        {image.url ? <span className="web-ui-image__url">{image.url}</span> : null}
      </figcaption>
    </figure>
  )
}

function FunctionIdLabel({ functionId }: { functionId: string }) {
  const tail = functionId.startsWith('web::')
    ? functionId.slice('web::'.length)
    : functionId
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>web::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{tail}</span>
    </>
  )
}

export function createWebImageRenderer(): FunctionTriggerRenderer {
  return {
    id: 'web/page.js#image-display',
    isMatch: (functionId) => functionId === 'web::fetch',
    tryRender: renderImage,
    tryRenderRunning: () => null,
    tryRenderPreview: () => null,
    FunctionIdLabel,
    metadata: { display: true },
  }
}
