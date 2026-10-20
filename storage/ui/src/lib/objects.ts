/**
 * Buckets and object bytes as the explorer reads them.
 *
 * A bucket comes from `storage::listBuckets`, parsed leniently so an older
 * worker (name and provider only) still lists. Bytes move one of two ways:
 * directly, from a presigned GET URL the browser fetches itself (images,
 * videos — seekable, the local server answers Range — and downloads), or
 * through the worker with `storage::getObject` when the bucket cannot sign
 * URLs (`direct_downloads: false`), in ranged 2 MiB reads on a worker that
 * has them and in one inline read of at most 10 MiB on one that doesn't.
 */

import type { Host } from '@iii-dev/console-ui'
import { errorCode } from './errors'

export interface BucketSummary {
  name: string
  provider: string
  bucket?: string
  region?: string
  endpoint_url?: string
  display?: { icon?: string; color?: string }
  uses_secrets?: boolean
  /** `false`: no presigned GET for this bucket; read through the worker. Absent on older workers. */
  direct_downloads?: boolean
  /** Set when the worker could not build the bucket; calls to it fail with `BUCKET_UNAVAILABLE`. */
  error?: string
  error_stage?: string
}

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value !== '' ? value : undefined
}

export function parseBucketSummary(raw: unknown): BucketSummary | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null
  const rec = raw as Record<string, unknown>
  const name = text(rec.name)
  if (!name) return null
  const display = typeof rec.display === 'object' && rec.display !== null ? (rec.display as Record<string, unknown>) : null
  const color = text(display?.color)
  return {
    name,
    provider: text(rec.provider) ?? 'unknown',
    bucket: text(rec.bucket),
    region: text(rec.region),
    endpoint_url: text(rec.endpoint_url),
    display: display ? { icon: text(display.icon), color: color && /^#[0-9A-Fa-f]{6}$/.test(color) ? color : undefined } : undefined,
    uses_secrets: rec.uses_secrets === true,
    direct_downloads: typeof rec.direct_downloads === 'boolean' ? rec.direct_downloads : undefined,
    error: text(rec.error),
    error_stage: text(rec.error_stage),
  }
}

export function parseBucketList(raw: unknown): BucketSummary[] {
  const rows =
    typeof raw === 'object' && raw !== null && Array.isArray((raw as { buckets?: unknown }).buckets)
      ? ((raw as { buckets: unknown[] }).buckets as unknown[])
      : []
  return rows.map(parseBucketSummary).filter((b): b is BucketSummary => b !== null)
}

/** The explorer's provider line: `S3-compatible` for S3 with a custom endpoint. */
export function bucketProviderLabel(bucket: BucketSummary): string {
  switch (bucket.provider) {
    case 'local':
      return 'Local'
    case 's3':
      return bucket.endpoint_url ? 'S3-compatible' : 'S3'
    case 'r2':
      return 'R2'
    case 'gcs':
      return 'GCS'
    default:
      return bucket.provider
  }
}

/* ---------------- previews ---------------- */

export type PreviewKind = 'image' | 'video'

const IMAGE_TYPES = /^image\/(?:png|jpe?g|pjpeg|gif|webp|avif|bmp|svg\+xml|x-icon|vnd\.microsoft\.icon)$/i
const VIDEO_TYPES = /^video\/(?:mp4|webm|ogg|quicktime|x-m4v)$/i
const GENERIC_TYPES = /^(?:application|binary)\/octet-stream$/i
const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'jfif', 'gif', 'webp', 'avif', 'bmp', 'svg', 'ico'])
const VIDEO_EXTENSIONS = new Set(['mp4', 'm4v', 'webm', 'ogv', 'mov'])

/**
 * Whether the browser can show an object inline, from its content type, or
 * from its extension when the type says nothing (`application/octet-stream`,
 * which S3 assigns to anything uploaded without one).
 */
export function previewKind(contentType: string | null | undefined, key: string): PreviewKind | null {
  const type = (contentType ?? '').split(';')[0].trim()
  if (type && !GENERIC_TYPES.test(type)) {
    if (IMAGE_TYPES.test(type)) return 'image'
    if (VIDEO_TYPES.test(type)) return 'video'
    return null
  }
  const leaf = key.slice(key.lastIndexOf('/') + 1)
  const dot = leaf.lastIndexOf('.')
  const ext = dot > 0 ? leaf.slice(dot + 1).toLowerCase() : ''
  if (IMAGE_EXTENSIONS.has(ext)) return 'image'
  if (VIDEO_EXTENSIONS.has(ext)) return 'video'
  return null
}

/** An http:// preview inside an https:// console is mixed content: the browser blocks or upgrades it. */
export function mixedContent(pageProtocol: string, url: string): boolean {
  return pageProtocol === 'https:' && /^http:\/\//i.test(url)
}

/* ---------------- relay reads ---------------- */

/** `storage::getObject`'s inline cap, on the bytes one call returns. */
export const INLINE_CAP = 10 * 1024 * 1024
export const RELAY_CHUNK = 2 * 1024 * 1024

/** The ranged reads that cover `size` bytes. An empty object is one read of nothing. */
export function chunkPlan(size: number, chunk = RELAY_CHUNK): Array<{ offset: number; length: number }> {
  if (size <= 0) return [{ offset: 0, length: 0 }]
  const out: Array<{ offset: number; length: number }> = []
  for (let offset = 0; offset < size; offset += chunk) out.push({ offset, length: Math.min(chunk, size - offset) })
  return out
}

export function decodeBase64(b64: string): Uint8Array<ArrayBuffer> {
  const binary = atob(b64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
  return bytes
}

interface GetObjectResponse {
  body_base64?: string
  content_type?: string
  size?: number
  offset?: number
  length?: number
}

export class RelayError extends Error {}

export const RELAY_TOO_OLD =
  'This object is larger than 10 MiB and this bucket cannot sign download links; update the storage worker to read it in parts.'

/**
 * The object's bytes through the worker: ranged reads on a worker that has
 * them, one inline read (≤ 10 MiB) on one that doesn't — told apart by the
 * response, which only a ranged worker answers with `offset`/`length`.
 */
export async function readThroughWorker(
  host: Host,
  bucket: string,
  key: string,
  size: number,
  options: {
    contentType?: string
    onProgress?: (done: number, total: number) => void
    signal?: AbortSignal
    chunk?: number
  } = {},
): Promise<Blob> {
  const parts: Uint8Array<ArrayBuffer>[] = []
  let contentType = options.contentType
  let done = 0
  for (const { offset, length } of chunkPlan(size, options.chunk)) {
    if (options.signal?.aborted) throw new RelayError('Cancelled.')
    let response: GetObjectResponse
    try {
      response = await host.iii.trigger<GetObjectResponse>(
        'storage::getObject',
        size > 0 ? { bucket, key, offset, length } : { bucket, key },
        { timeoutMs: 60_000 },
      )
    } catch (error) {
      if (errorCode(error) === 'OBJECT_TOO_LARGE') throw new RelayError(RELAY_TOO_OLD)
      throw error
    }
    const bytes = decodeBase64(response.body_base64 ?? '')
    contentType ??= response.content_type
    const ranged = typeof response.offset === 'number' || typeof response.length === 'number'
    if (!ranged) {
      // An older worker ignored the range and sent the whole object.
      return new Blob([bytes], { type: contentType ?? 'application/octet-stream' })
    }
    parts.push(bytes)
    done += bytes.length
    options.onProgress?.(done, size)
    if (bytes.length === 0 && length > 0) break
  }
  return new Blob(parts, { type: contentType ?? 'application/octet-stream' })
}

/** `attachment; filename="…"` for a presigned download, quotes dropped and non-ASCII spelled per RFC 5987. */
export function attachmentDisposition(filename: string): string {
  const plain = filename.replaceAll('"', '').replace(/[^\x20-\x7e]/g, '_')
  const encoded = encodeURIComponent(filename).replace(/['()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`)
  return plain === filename ? `attachment; filename="${plain}"` : `attachment; filename="${plain}"; filename*=UTF-8''${encoded}`
}
