/* Streaming a file's bytes into the page in bounded chunks over
   `shell::workspace::read-bytes`, assembled into a Blob. The engine socket
   never sees more than one chunk's base64 at a time, and the page keeps
   one copy of the bytes (the Blob) instead of a base64 string plus a data
   URL. */

import type { Host } from '@iii-dev/console-ui'
import { IMAGE_CHUNK_BYTES } from './large-file'

export interface ReadBytesChunk {
  path: string
  size: number
  offset: number
  length: number
  content: string
  mtime: number
  eof: boolean
}

export interface ReadBytesOptions {
  chunkBytes?: number
  /** Concurrent chunk requests in flight. */
  parallel?: number
  signal?: AbortSignal
  onProgress?: (received: number, total: number) => void
  /** Refuse files over this many bytes before reading anything. */
  maxBytes?: number
}

export interface FileBytes {
  blob: Blob
  size: number
  mtime: number
}

export function decodeBase64(content: string): Uint8Array {
  const binary = atob(content)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
  return bytes
}

/** The byte ranges that cover `size` in `chunk`-sized pieces. */
export function chunkRanges(size: number, chunk: number): { offset: number; length: number }[] {
  const ranges: { offset: number; length: number }[] = []
  for (let offset = 0; offset < size; offset += chunk) {
    ranges.push({ offset, length: Math.min(chunk, size - offset) })
  }
  return ranges
}

function readChunk(host: Host, path: string, offset: number, length: number): Promise<ReadBytesChunk> {
  return host.iii.trigger<ReadBytesChunk>('shell::workspace::read-bytes', {
    path,
    offset,
    length,
  })
}

export async function readFileBytes(
  host: Host,
  path: string,
  mime: string,
  options: ReadBytesOptions = {},
): Promise<FileBytes> {
  const chunk = options.chunkBytes ?? IMAGE_CHUNK_BYTES
  const parallel = Math.max(1, options.parallel ?? 2)
  const first = await readChunk(host, path, 0, chunk)
  if (options.maxBytes !== undefined && first.size > options.maxBytes) {
    throw new Error(`file is ${first.size} bytes, over the ${options.maxBytes} byte preview limit`)
  }
  const parts: Uint8Array[] = new Array(Math.max(1, Math.ceil(first.size / chunk)))
  parts[0] = decodeBase64(first.content)
  let received = first.length
  options.onProgress?.(received, first.size)
  const ranges = chunkRanges(first.size, chunk).slice(1)
  let cursor = 0
  const worker = async () => {
    while (cursor < ranges.length) {
      if (options.signal?.aborted) throw new Error('aborted')
      const index = cursor++
      const range = ranges[index]
      const part = await readChunk(host, path, range.offset, range.length)
      if (part.mtime !== first.mtime || part.size !== first.size) {
        throw new Error('file changed while it was being read')
      }
      parts[index + 1] = decodeBase64(part.content)
      received += part.length
      options.onProgress?.(received, first.size)
    }
  }
  await Promise.all(Array.from({ length: Math.min(parallel, ranges.length) }, worker))
  if (options.signal?.aborted) throw new Error('aborted')
  return {
    blob: new Blob(parts as BlobPart[], { type: mime }),
    size: first.size,
    mtime: first.mtime,
  }
}

/** Object URLs the page hands to `<img>`: created per cache entry and
    revoked when the entry goes, so a closed tab frees its bytes. */
export function createObjectUrlRegistry(): {
  create(blob: Blob): string
  release(url: string | null | undefined): void
  releaseAll(): void
} {
  const urls = new Set<string>()
  return {
    create(blob) {
      const url = URL.createObjectURL(blob)
      urls.add(url)
      return url
    },
    release(url) {
      if (!url || !urls.has(url)) return
      urls.delete(url)
      URL.revokeObjectURL(url)
    },
    releaseAll() {
      for (const url of urls) URL.revokeObjectURL(url)
      urls.clear()
    },
  }
}
