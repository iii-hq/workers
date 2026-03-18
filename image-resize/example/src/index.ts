import { http, Logger } from 'iii-sdk'
import { useApi } from './hooks'
import { iii } from './iii'

/** Detect image format from the first bytes (magic numbers). */
function detectFormat(buf: Buffer): 'jpeg' | 'png' | 'webp' {
  if (buf[0] === 0xff && buf[1] === 0xd8) return 'jpeg'
  if (buf[0] === 0x89 && buf[1] === 0x50 && buf[2] === 0x4e && buf[3] === 0x47) return 'png'
  if (buf[8] === 0x57 && buf[9] === 0x45 && buf[10] === 0x42 && buf[11] === 0x50) return 'webp'
  return 'jpeg' // fallback
}

// Serialize access to the resizer to avoid concurrent invocation issues.
// The engine architecture supports parallel invocations, but there's a known
// issue with concurrent channel-based function calls. Remove this once fixed.
let pending: Promise<unknown> = Promise.resolve()
function serialized<T>(fn: () => Promise<T>): Promise<T> {
  const next = pending.then(fn, fn)
  pending = next
  return next
}

/**
 * Send an image to the image_resize::resize function via SDK channels
 * and return the thumbnail bytes.
 *
 * 1. Create two channels (input for image data, output for thumbnail data)
 * 2. Trigger the resize function with channel refs + metadata
 * 3. Write image bytes to input channel
 * 4. Read thumbnail metadata (text frame) + bytes (binary) from output channel
 * 5. Return result
 */
async function processImage(
  imageBuffer: Buffer,
  opts: { format: string; outputFormat: string; width: number; height: number; strategy: string },
): Promise<{ thumbnail: Buffer; metadata: Record<string, unknown> }> {
  // Create two channels: one for sending the image, one for receiving the thumbnail
  const inputChannel = await iii.createChannel()
  const outputChannel = await iii.createChannel()

  // Write image bytes to the input channel, then close it.
  // The SDK's ChannelWriter flushes all data before sending the close frame.
  inputChannel.writer.stream.end(imageBuffer)

  // Trigger the resize function
  const triggerPromise = iii.trigger({
    function_id: 'image_resize::resize',
    payload: {
      input_channel: inputChannel.readerRef,
      output_channel: outputChannel.writerRef,
      metadata: {
        format: opts.format,
        output_format: opts.outputFormat,
        width: 0,
        height: 0,
        target_width: opts.width,
        target_height: opts.height,
        strategy: opts.strategy,
      },
    },
    timeoutMs: 30_000,
  })

  // Set up both listeners BEFORE awaiting — the reader WebSocket only connects
  // when the stream starts flowing (on 'data'), so we must register both the
  // text-message callback and the stream listener concurrently to avoid deadlock.
  const metadataPromise = new Promise<Record<string, unknown>>((resolve) => {
    outputChannel.reader.onMessage((msg) => {
      resolve(JSON.parse(msg))
    })
  })

  const chunks: Buffer[] = []
  const thumbnailPromise = new Promise<Buffer>((resolve, reject) => {
    outputChannel.reader.stream.on('data', (chunk: Buffer) => {
      chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
    })
    outputChannel.reader.stream.on('end', () => resolve(Buffer.concat(chunks)))
    outputChannel.reader.stream.on('error', reject)
  })

  const [metadata, thumbnail] = await Promise.all([metadataPromise, thumbnailPromise])

  // Wait for the trigger to complete
  await triggerPromise

  // Explicitly close all channel endpoints to free engine resources
  outputChannel.reader.stream.destroy()
  inputChannel.writer.close()

  return { thumbnail, metadata }
}

// ── Health check ──────────────────────────────────────

useApi(
  {
    api_path: '/health',
    http_method: 'GET',
    description: 'Health check',
  },
  async () => ({
    status_code: 200,
    body: { status: 'ok', service: 'image-resize-demo' },
    headers: { 'Content-Type': 'application/json' },
  }),
)

// ── Thumbnail endpoint ────────────────────────────────

useApi(
  {
    api_path: '/thumbnail',
    http_method: 'POST',
    description: 'Generate a thumbnail from a base64-encoded image via the image-resize module',
    metadata: { tags: ['image', 'thumbnail'] },
  },
  async (req, logger) => {
    const {
      image,
      width = 200,
      height = 200,
      strategy = 'scale-to-fit',
      format,
      outputFormat = 'jpeg',
    } = req.body as {
      image: string
      width?: number
      height?: number
      strategy?: 'scale-to-fit' | 'crop-to-fit'
      format?: 'jpeg' | 'png' | 'webp'
      outputFormat?: 'jpeg' | 'png' | 'webp'
    }

    if (!image) {
      return {
        status_code: 400,
        body: { error: 'Missing "image" field (base64-encoded image data)' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    // Detect input format from image bytes if not explicitly provided
    const imageBuffer = Buffer.from(image, 'base64')
    const inputFormat = format ?? detectFormat(imageBuffer)

    logger.info('Processing thumbnail request', { inputFormat, outputFormat, width, height, strategy })

    try {
      const { thumbnail, metadata } = await serialized(() =>
        processImage(imageBuffer, { format: inputFormat, outputFormat, width, height, strategy }),
      )

      logger.info('Thumbnail generated', {
        format: metadata.format,
        width: metadata.width,
        height: metadata.height,
        size: thumbnail.length,
      })

      return {
        status_code: 200,
        body: {
          thumbnail: thumbnail.toString('base64'),
          format: metadata.format,
          width: metadata.width,
          height: metadata.height,
          size: thumbnail.length,
        },
        headers: { 'Content-Type': 'application/json' },
      }
    } catch (err) {
      logger.error('Thumbnail generation failed', { error: String(err) })
      return {
        status_code: 500,
        body: { error: `Thumbnail generation failed: ${err}` },
        headers: { 'Content-Type': 'application/json' },
      }
    }
  },
)


// ── URL-based image resize endpoint (Next.js-style) ─────

const MAX_FETCH_SIZE = 10 * 1024 * 1024 // 10 MB
const FETCH_TIMEOUT_MS = 10_000
const MIME_TYPES: Record<string, string> = {
  jpeg: 'image/jpeg',
  png: 'image/png',
  webp: 'image/webp',
}

const imageLogger = new Logger(undefined, 'api::get::/image')

function sendJsonError(res: { status: (n: number) => void; headers: (h: Record<string, string>) => void; stream: NodeJS.WritableStream; close: () => void }, statusCode: number, error: string) {
  res.status(statusCode)
  res.headers({ 'content-type': 'application/json' })
  res.stream.end(JSON.stringify({ error }))
  res.close()
}

{
  const function_id = 'api::get::/image'

  iii.registerFunction(
    { id: function_id, metadata: { tags: ['image', 'resize', 'url'] } },
    http(async (req, res) => {
      const qp = req.query_params ?? {}
      const str = (v: string | string[] | undefined): string | undefined =>
        Array.isArray(v) ? v[0] : v

      const url = str(qp.url)
      const w = Number(str(qp.w)) || 200
      const h = Number(str(qp.h)) || 200
      const strategy = str(qp.strategy) || 'scale-to-fit'
      const format = str(qp.format) as 'jpeg' | 'png' | 'webp' | undefined
      const outputFormat = (str(qp.format) as 'jpeg' | 'png' | 'webp') || 'jpeg'

      // ── Validate URL ──
      if (!url) { sendJsonError(res, 400, 'Missing "url" query parameter'); return }

      let parsed: URL
      try { parsed = new URL(url) } catch {
        sendJsonError(res, 400, 'Invalid URL'); return
      }

      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        sendJsonError(res, 400, 'Only http:// and https:// URLs are allowed'); return
      }

      // ── Validate dimensions ──
      if (w < 1 || w > 4096 || h < 1 || h > 4096) {
        sendJsonError(res, 400, 'Width and height must be between 1 and 4096'); return
      }

      if (strategy !== 'scale-to-fit' && strategy !== 'crop-to-fit') {
        sendJsonError(res, 400, 'Strategy must be "scale-to-fit" or "crop-to-fit"'); return
      }

      // ── Fetch the remote image ──
      imageLogger.info('Fetching remote image', { url, w, h, strategy, outputFormat })

      let imageBuffer: Buffer
      try {
        const controller = new AbortController()
        const timeout = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS)

        const fetchRes = await fetch(url, { signal: controller.signal })
        clearTimeout(timeout)

        if (!fetchRes.ok) {
          sendJsonError(res, 502, `Failed to fetch image: upstream returned ${fetchRes.status}`); return
        }

        const contentLength = Number(fetchRes.headers.get('content-length') || 0)
        if (contentLength > MAX_FETCH_SIZE) {
          sendJsonError(res, 400, `Image too large (${contentLength} bytes). Maximum is ${MAX_FETCH_SIZE} bytes.`); return
        }

        const arrayBuf = await fetchRes.arrayBuffer()
        if (arrayBuf.byteLength > MAX_FETCH_SIZE) {
          sendJsonError(res, 400, `Image too large (${arrayBuf.byteLength} bytes). Maximum is ${MAX_FETCH_SIZE} bytes.`); return
        }

        imageBuffer = Buffer.from(arrayBuf)
      } catch (err) {
        const message = err instanceof Error && err.name === 'AbortError'
          ? 'Upstream image fetch timed out'
          : `Failed to fetch image: ${err}`
        sendJsonError(res, 502, message); return
      }

      // ── Detect format & process ──
      const inputFormat = format ?? detectFormat(imageBuffer)

      try {
        const { thumbnail, metadata } = await serialized(() =>
          processImage(imageBuffer, { format: inputFormat, outputFormat, width: w, height: h, strategy }),
        )

        imageLogger.info('URL image resized', {
          url,
          format: metadata.format,
          width: metadata.width,
          height: metadata.height,
          size: thumbnail.length,
        })

        // Return binary image directly
        const mimeType = MIME_TYPES[metadata.format as string] ?? 'application/octet-stream'
        res.status(200)
        res.headers({
          'content-type': mimeType,
          'content-length': String(thumbnail.length),
          'cache-control': 'public, max-age=86400',
        })
        res.stream.end(thumbnail)
        res.close()
      } catch (err) {
        imageLogger.error('URL image resize failed', { error: String(err) })
        sendJsonError(res, 500, `Image processing failed: ${err}`)
      }
    }),
  )

  iii.registerTrigger({
    type: 'http',
    function_id,
    config: {
      api_path: '/image',
      http_method: 'GET',
      description: 'Resize a remote image by URL — similar to Next.js /_next/image',
      metadata: { tags: ['image', 'resize', 'url'] },
    },
  })
}

console.log('Image resize demo worker started — registering endpoints...')
