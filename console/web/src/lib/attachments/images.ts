/**
 * Image attachments, as pictures rather than as prose about pictures.
 *
 * An image pasted or dropped into the composer used to reach the model as
 * nothing: the chip rendered a thumbnail, and the send forwarded text blocks
 * only. Everything underneath was already in place — the harness carries
 * `ContentBlock::Image`, and the Anthropic and OpenAI providers both map it —
 * so the picture was being dropped one layer above the plumbing that could
 * have delivered it.
 *
 * Two things happen here before an image goes out. It is checked against the
 * formats every vision model accepts, because a `.heic` from a phone is a file
 * no provider will decode and a block saying so is more useful than an API
 * error. And it is downscaled when it is larger than a model can take: a
 * screenshot on a retina display is routinely eight megabytes, and no answer
 * gets better for the pixels beyond the long-edge ceiling.
 */

import type { Attachment } from '@/types/chat'
import {
  type AttachmentFailure,
  type AttachmentImageBlock,
  type AttachmentReadSummary,
  extensionOf,
  failureBlock,
  fileToBase64,
  formatBytes,
  reportDropped,
} from './shared'

/** Images sent per message. Each one costs real tokens on arrival. */
export const MAX_IMAGES_PER_SEND = 8

/**
 * The formats every vision provider decodes. Anything else is refused by name
 * rather than sent and rejected by the API.
 */
export const SUPPORTED_IMAGE_MIME = new Set([
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
])

/**
 * Longest edge kept when an image is downscaled. Beyond roughly this size the
 * models resize server-side anyway, so the extra pixels only cost upload time
 * and tokens.
 */
export const MAX_IMAGE_EDGE = 1568

/**
 * Bytes above which an image is downscaled before sending. Providers cap the
 * encoded payload at around five megabytes, and base64 inflates by a third, so
 * the ceiling here is on the raw file.
 */
export const MAX_IMAGE_BYTES = 3.5 * 1024 * 1024

/**
 * Hard refusal ceiling. Past this the browser is decoding a file large enough
 * to stall the tab, and the answer is to point at the file rather than paste
 * it.
 */
export const MAX_SOURCE_IMAGE_BYTES = 32 * 1024 * 1024

export interface ExpandedImages {
  /** Native image content blocks for the outgoing message. */
  images: AttachmentImageBlock[]
  /** Blocks explaining any image that could NOT be sent as a picture. */
  blocks: string[]
  read: AttachmentReadSummary[]
  failures: AttachmentFailure[]
}

/** Whether an attachment is an image, by declared type or by extension. */
export function isImageAttachment(attachment: Attachment): boolean {
  if (attachment.type.startsWith('image/')) return true
  return IMAGE_EXTENSIONS.has(extensionOf(attachment.name))
}

const IMAGE_EXTENSIONS = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'heic',
  'heif',
  'bmp',
  'tif',
  'tiff',
  'avif',
])

/** The MIME type to send under, from the declared type or the extension. */
export function imageMimeOf(attachment: Attachment): string {
  if (attachment.type.startsWith('image/')) return attachment.type
  const ext = extensionOf(attachment.name)
  if (ext === 'jpg' || ext === 'jpeg') return 'image/jpeg'
  return ext ? `image/${ext}` : 'application/octet-stream'
}

/**
 * The dimensions an image is downscaled to: the same aspect ratio, longest
 * edge at the ceiling. An image already inside the ceiling keeps its size —
 * upscaling a small screenshot would add bytes and no detail.
 */
export function fitWithin(
  width: number,
  height: number,
  edge = MAX_IMAGE_EDGE,
): { width: number; height: number } {
  const longest = Math.max(width, height)
  if (longest <= edge) return { width, height }
  const scale = edge / longest
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  }
}

/** Whether this file has to be re-encoded before it can be sent. */
export function needsDownscale(file: { size: number }): boolean {
  return file.size > MAX_IMAGE_BYTES
}

/**
 * The pixel dimensions in an image's own header, without decoding it.
 *
 * Bytes are a poor proxy for size: a screenshot of a mostly-flat UI compresses
 * to a few hundred kilobytes at eight thousand pixels wide, sails under the
 * byte ceiling, and then costs a fortune in tokens for detail no model uses.
 * Reading the header is cheap enough to do for every image and needs no canvas,
 * which keeps the decision testable.
 *
 * `null` for a format not parsed here — the byte ceiling stays the backstop.
 */
export function dimensionsOf(
  bytes: Uint8Array,
): { width: number; height: number } | null {
  // Read the integers by hand rather than through a DataView: a Uint8Array's
  // buffer may be a SharedArrayBuffer as far as the types are concerned, and
  // these are four fields in three formats.
  const u16 = (at: number) => (bytes[at] << 8) | bytes[at + 1]
  const u16le = (at: number) => bytes[at] | (bytes[at + 1] << 8)
  const u32 = (at: number) =>
    bytes[at] * 0x1000000 +
    ((bytes[at + 1] << 16) | (bytes[at + 2] << 8) | bytes[at + 3])

  // PNG: IHDR is always the first chunk, width and height at a fixed offset.
  if (bytes.length > 24 && bytes[0] === 0x89 && bytes[1] === 0x50) {
    return { width: u32(16), height: u32(20) }
  }

  // GIF: little-endian, straight after the signature.
  if (bytes.length > 10 && bytes[0] === 0x47 && bytes[1] === 0x49) {
    return { width: u16le(6), height: u16le(8) }
  }

  // JPEG: walk the segment chain to the start-of-frame, which is the only
  // marker carrying the dimensions.
  if (bytes.length > 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    let offset = 2
    while (offset + 9 < bytes.length) {
      if (bytes[offset] !== 0xff) return null
      const marker = bytes[offset + 1]
      // SOF0-SOF15, minus the four that are not frame headers.
      const isFrame =
        marker >= 0xc0 &&
        marker <= 0xcf &&
        marker !== 0xc4 &&
        marker !== 0xc8 &&
        marker !== 0xcc
      if (isFrame) return { height: u16(offset + 5), width: u16(offset + 7) }
      offset += 2 + u16(offset + 2)
    }
  }

  return null
}

/** Whether an image is larger than the long-edge ceiling. */
export function exceedsEdge(bytes: Uint8Array, edge = MAX_IMAGE_EDGE): boolean {
  const size = dimensionsOf(bytes)
  return size !== null && Math.max(size.width, size.height) > edge
}

/**
 * Re-encode an oversized image at the long-edge ceiling.
 *
 * Everything runs in the browser: the bytes are already here, and a round trip
 * to a worker to shrink a screenshot would be slower than decoding it in
 * place. Injectable so the decision logic above stays testable without a
 * canvas.
 */
export type Downscaler = (
  file: File,
) => Promise<{ blob: Blob; mime: string } | null>

const downscaleInBrowser: Downscaler = async (file) => {
  if (typeof createImageBitmap !== 'function') return null
  let bitmap: ImageBitmap
  try {
    bitmap = await createImageBitmap(file)
  } catch {
    return null
  }
  try {
    const { width, height } = fitWithin(bitmap.width, bitmap.height)
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const context = canvas.getContext('2d')
    if (!context) return null
    context.drawImage(bitmap, 0, 0, width, height)
    // JPEG for photographs and screenshots alike: a PNG re-encode of a
    // photograph is frequently larger than the original, which is the opposite
    // of the point.
    const blob = await new Promise<Blob | null>((resolve) =>
      canvas.toBlob(resolve, 'image/jpeg', 0.85),
    )
    return blob ? { blob, mime: 'image/jpeg' } : null
  } finally {
    bitmap.close()
  }
}

/**
 * Turn every attached image into an image content block.
 *
 * Attachments without their underlying `File` are skipped silently: a
 * conversation reloaded from history keeps the chip, not the bytes.
 */
export async function expandImageAttachments(
  attachments: Attachment[],
  downscale: Downscaler = downscaleInBrowser,
): Promise<ExpandedImages> {
  const candidates = attachments.filter((a) => isImageAttachment(a) && a.file)
  if (candidates.length === 0)
    return { images: [], blocks: [], read: [], failures: [] }

  const images: AttachmentImageBlock[] = []
  const blocks: string[] = []
  const read: AttachmentReadSummary[] = []
  const failures: AttachmentFailure[] = []

  const refuse = (attachment: Attachment, reason: string) => {
    blocks.push(failureBlock(attachment.name, reason))
    failures.push({ name: attachment.name, reason })
  }

  for (const attachment of candidates.slice(0, MAX_IMAGES_PER_SEND)) {
    const mime = imageMimeOf(attachment)
    const file = attachment.file as File

    if (attachment.size > MAX_SOURCE_IMAGE_BYTES) {
      refuse(
        attachment,
        `${formatBytes(attachment.size)} is too large to send from the composer; point at the file on disk instead`,
      )
      continue
    }

    const unreadableFormat = !SUPPORTED_IMAGE_MIME.has(mime)
    // Both ceilings matter, and neither implies the other: a photograph busts
    // the byte limit at a sane resolution, while a flat-coloured screenshot
    // eight thousand pixels wide compresses under it and still costs tokens for
    // detail no model uses.
    const tooLarge =
      needsDownscale(file) ||
      exceedsEdge(new Uint8Array(await file.arrayBuffer()))
    // One re-encode covers all three problems: it lands on JPEG at the
    // long-edge ceiling, which is a format every model reads and a size every
    // model takes.
    const converted =
      unreadableFormat || tooLarge ? await downscale(file) : null

    if (!converted && (unreadableFormat || tooLarge)) {
      refuse(
        attachment,
        unreadableFormat
          ? `${mime} is not a format a model can read, and this browser could not convert it`
          : `${formatBytes(attachment.size)} or ${MAX_IMAGE_EDGE}px is over the limit for one image and it could not be resized here`,
      )
      continue
    }

    const payload: Blob = converted?.blob ?? file
    images.push({
      type: 'image',
      mime: converted?.mime ?? mime,
      data: await fileToBase64(payload),
    })
    read.push({
      id: attachment.id,
      label: `${attachment.name} · ${formatBytes(payload.size)}${converted ? ' · resized' : ''}`,
    })
  }

  reportDropped(
    candidates.slice(MAX_IMAGES_PER_SEND),
    `only ${MAX_IMAGES_PER_SEND} images are sent per message`,
    { blocks, failures },
  )

  return { images, blocks, read, failures }
}
