import { Writable } from 'node:stream'
import { http } from '@iii-dev/helpers/http'
import { InputStream, OutputStream } from 'iii-sdk'
import { observeRoute, useApi } from './hooks'
import { iii } from './iii'

// Serialize access to the resizer to avoid concurrent invocation issues.
// The engine architecture supports parallel invocations, but there's a known
// issue with concurrent channel-based function calls. Remove this once fixed.
let pending: Promise<unknown> = Promise.resolve()
function serialized<T>(fn: () => Promise<T>): Promise<T> {
  const next = pending.then(fn, fn)
  pending = next
  return next
}

type ThumbnailMetadata = {
  format: string
  width: number
  height: number
  source_width: number
  source_height: number
  strategy: string
}

/**
 * Resize an image with image_resize::resize and return the thumbnail bytes
 * and the call's result. The worker detects the input format from the bytes;
 * the call resolves once the sink holds the whole thumbnail. A bad image or
 * option rejects with code INVALID_ARGS.
 */
async function processImage(
  imageBuffer: Buffer,
  opts: { outputFormat: string; width: number; height: number; strategy: string },
): Promise<{ thumbnail: Buffer; metadata: ThumbnailMetadata }> {
  const chunks: Buffer[] = []
  const metadata: ThumbnailMetadata = await iii.call({
    function_id: 'image_resize::resize',
    payload: {
      input_channel: InputStream.from(imageBuffer),
      output_channel: OutputStream.to(
        new Writable({
          write(chunk, _enc, done) {
            chunks.push(chunk)
            done()
          },
        }),
      ),
      // The worker rejects any other metadata key.
      metadata: {
        output_format: opts.outputFormat,
        target_width: opts.width,
        target_height: opts.height,
        strategy: opts.strategy,
      },
    },
    timeoutMs: 30_000,
  })
  return { thumbnail: Buffer.concat(chunks), metadata }
}

/** INVALID_ARGS is the caller's fault (bad image or option): 400, anything else 500. */
const failureStatus = (err: unknown) => ((err as { code?: string } | undefined)?.code === 'INVALID_ARGS' ? 400 : 500)

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
      outputFormat = 'jpeg',
    } = req.body as {
      image: string
      width?: number
      height?: number
      strategy?: 'scale-to-fit' | 'crop-to-fit'
      outputFormat?: 'jpeg' | 'png' | 'webp'
    }

    if (!image) {
      return {
        status_code: 400,
        body: { error: 'Missing "image" field (base64-encoded image data)' },
        headers: { 'Content-Type': 'application/json' },
      }
    }

    const imageBuffer = Buffer.from(image, 'base64')

    logger.info('Processing thumbnail request', { outputFormat, width, height, strategy })

    try {
      const { thumbnail, metadata } = await serialized(() =>
        processImage(imageBuffer, { outputFormat, width, height, strategy }),
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
        status_code: failureStatus(err),
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

const imageLogger = console

function sendJsonError(
  res: {
    status: (n: number) => void
    headers: (h: Record<string, string>) => void
    stream: NodeJS.WritableStream
    close: () => void
  },
  statusCode: number,
  error: string,
) {
  res.status(statusCode)
  res.headers({ 'content-type': 'application/json' })
  res.stream.end(JSON.stringify({ error }))
  res.close()
}

{
  const function_id = 'api::get::/image'

  iii.registerFunction(
    function_id,
    http(async (req, res) => {
      const qp = req.query_params ?? {}
      const str = (v: string | string[] | undefined): string | undefined => (Array.isArray(v) ? v[0] : v)

      const url = str(qp.url)
      const w = Number(str(qp.w)) || 200
      const h = Number(str(qp.h)) || 200
      const strategy = str(qp.strategy) || 'scale-to-fit'
      const outputFormat = (str(qp.format) as 'jpeg' | 'png' | 'webp') || 'jpeg'

      // ── Validate URL ──
      if (!url) {
        sendJsonError(res, 400, 'Missing "url" query parameter')
        return
      }

      let parsed: URL
      try {
        parsed = new URL(url)
      } catch {
        sendJsonError(res, 400, 'Invalid URL')
        return
      }

      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        sendJsonError(res, 400, 'Only http:// and https:// URLs are allowed')
        return
      }

      // ── Validate dimensions ──
      if (w < 1 || w > 4096 || h < 1 || h > 4096) {
        sendJsonError(res, 400, 'Width and height must be between 1 and 4096')
        return
      }

      if (strategy !== 'scale-to-fit' && strategy !== 'crop-to-fit') {
        sendJsonError(res, 400, 'Strategy must be "scale-to-fit" or "crop-to-fit"')
        return
      }

      // ── Fetch the remote image ──
      imageLogger.info('Fetching remote image', { url, w, h, strategy, outputFormat })

      let imageBuffer: Buffer
      try {
        // The signal also covers the body read below.
        const fetchRes = await fetch(url, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS) })

        if (!fetchRes.ok) {
          sendJsonError(res, 502, `Failed to fetch image: upstream returned ${fetchRes.status}`)
          return
        }

        const contentLength = Number(fetchRes.headers.get('content-length') || 0)
        if (contentLength > MAX_FETCH_SIZE) {
          sendJsonError(res, 400, `Image too large (${contentLength} bytes). Maximum is ${MAX_FETCH_SIZE} bytes.`)
          return
        }

        const arrayBuf = await fetchRes.arrayBuffer()
        if (arrayBuf.byteLength > MAX_FETCH_SIZE) {
          sendJsonError(res, 400, `Image too large (${arrayBuf.byteLength} bytes). Maximum is ${MAX_FETCH_SIZE} bytes.`)
          return
        }

        imageBuffer = Buffer.from(arrayBuf)
      } catch (err) {
        const message =
          err instanceof Error && err.name === 'TimeoutError'
            ? 'Upstream image fetch timed out'
            : `Failed to fetch image: ${err}`
        sendJsonError(res, 502, message)
        return
      }

      // ── Process ──
      try {
        const { thumbnail, metadata } = await serialized(() =>
          processImage(imageBuffer, { outputFormat, width: w, height: h, strategy }),
        )

        imageLogger.info('URL image resized', {
          url,
          format: metadata.format,
          width: metadata.width,
          height: metadata.height,
          size: thumbnail.length,
        })

        // Return binary image directly
        const mimeType = MIME_TYPES[metadata.format] ?? 'application/octet-stream'
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
        sendJsonError(res, failureStatus(err), `Image processing failed: ${err}`)
      }
    }),
    {
      description: 'Resize a remote image by URL — similar to Next.js /_next/image',
      metadata: { tags: ['image', 'resize', 'url'] },
    },
  )

  observeRoute(function_id, 'GET', '/image')
}

console.log('Image resize demo worker started — registering endpoints...')
