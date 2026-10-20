import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import {
  attachmentDisposition,
  bucketProviderLabel,
  chunkPlan,
  decodeBase64,
  INLINE_CAP,
  mixedContent,
  parseBucketList,
  parseBucketSummary,
  previewKind,
  RELAY_CHUNK,
  RELAY_TOO_OLD,
  readThroughWorker,
} from './objects'

function b64(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary)
}

describe('buckets', () => {
  it('reads an extended listBuckets row', () => {
    expect(
      parseBucketSummary({
        name: 'photos',
        provider: 's3',
        bucket: 'photos-prod',
        region: 'us-east-1',
        endpoint_url: 'http://minio:9000',
        display: { icon: 'cloud', color: '#2563EB' },
        uses_secrets: true,
        direct_downloads: true,
      }),
    ).toEqual({
      name: 'photos',
      provider: 's3',
      bucket: 'photos-prod',
      region: 'us-east-1',
      endpoint_url: 'http://minio:9000',
      display: { icon: 'cloud', color: '#2563EB' },
      uses_secrets: true,
      direct_downloads: true,
      error: undefined,
      error_stage: undefined,
    })
  })

  it('keeps an older worker’s rows and a bucket that failed to build', () => {
    expect(parseBucketList({ buckets: [{ name: 'a', provider: 'local' }, { provider: 's3' }, 'x'] })).toEqual([
      {
        name: 'a',
        provider: 'local',
        bucket: undefined,
        region: undefined,
        endpoint_url: undefined,
        display: undefined,
        uses_secrets: false,
        direct_downloads: undefined,
        error: undefined,
        error_stage: undefined,
      },
    ])
    const broken = parseBucketSummary({
      name: 'cdn',
      provider: 'r2',
      error: 'secret STORAGE_CDN_SECRET_ACCESS_KEY not found',
      error_stage: 'secrets',
      display: { color: 'red' },
    })
    expect(broken?.error).toBe('secret STORAGE_CDN_SECRET_ACCESS_KEY not found')
    expect(broken?.error_stage).toBe('secrets')
    expect(broken?.display).toEqual({ icon: undefined, color: undefined })
    expect(parseBucketList(null)).toEqual([])
    expect(parseBucketList({ buckets: 'no' })).toEqual([])
  })

  it('labels the provider, S3 with an endpoint as S3-compatible', () => {
    expect(bucketProviderLabel({ name: 'a', provider: 's3' })).toBe('S3')
    expect(bucketProviderLabel({ name: 'a', provider: 's3', endpoint_url: 'http://minio:9000' })).toBe('S3-compatible')
    expect(bucketProviderLabel({ name: 'a', provider: 'gcs' })).toBe('GCS')
    expect(bucketProviderLabel({ name: 'a', provider: 'local' })).toBe('Local')
    expect(bucketProviderLabel({ name: 'a', provider: 'azure' })).toBe('azure')
  })
})

describe('previews', () => {
  it('trusts a specific content type', () => {
    expect(previewKind('image/jpeg', 'a/b.bin')).toBe('image')
    expect(previewKind('image/svg+xml', 'logo')).toBe('image')
    expect(previewKind('image/webp; charset=binary', 'x')).toBe('image')
    expect(previewKind('video/mp4', 'clip')).toBe('video')
    expect(previewKind('video/webm', 'clip')).toBe('video')
    expect(previewKind('video/quicktime', 'clip.mov')).toBe('video')
    expect(previewKind('application/pdf', 'photo.jpg')).toBeNull()
    expect(previewKind('image/heic', 'IMG_0001.HEIC')).toBeNull()
    expect(previewKind('audio/mpeg', 'song.mp3')).toBeNull()
  })

  it('falls back to the extension when the type says nothing', () => {
    expect(previewKind('application/octet-stream', 'Fotos de férias/praia 🌊.JPG')).toBe('image')
    expect(previewKind('binary/octet-stream', 'videos/moov-at-end.mp4')).toBe('video')
    expect(previewKind('', 'clip.webm')).toBe('video')
    expect(previewKind(undefined, 'a.gif')).toBe('image')
    expect(previewKind('application/octet-stream', 'archive.zip')).toBeNull()
    expect(previewKind('application/octet-stream', '.jpg')).toBeNull()
    expect(previewKind('application/octet-stream', 'folder.jpg/file')).toBeNull()
  })

  it('spots an http link inside an https console', () => {
    expect(mixedContent('https:', 'http://100.70.145.81:49251/x')).toBe(true)
    expect(mixedContent('https:', 'https://bucket.s3.amazonaws.com/x')).toBe(false)
    expect(mixedContent('http:', 'http://localhost:9000/x')).toBe(false)
  })

  it('names a download for any file name, ASCII or not', () => {
    expect(attachmentDisposition('report.pdf')).toBe('attachment; filename="report.pdf"')
    expect(attachmentDisposition('say "hi".txt')).toBe(
      'attachment; filename="say hi.txt"; filename*=UTF-8\'\'say%20%22hi%22.txt',
    )
    const unicode = attachmentDisposition('praia 🌊.jpg')
    expect(unicode).toBe('attachment; filename="praia __.jpg"; filename*=UTF-8\'\'praia%20%F0%9F%8C%8A.jpg')
    // Every character is a valid header byte.
    expect(/^[\x20-\x7e]+$/.test(unicode)).toBe(true)
  })
})

describe('reads through the worker', () => {
  it('plans 2 MiB ranges and one read for an empty object', () => {
    expect(chunkPlan(0)).toEqual([{ offset: 0, length: 0 }])
    expect(chunkPlan(5)).toEqual([{ offset: 0, length: 5 }])
    expect(chunkPlan(RELAY_CHUNK * 2 + 1)).toEqual([
      { offset: 0, length: RELAY_CHUNK },
      { offset: RELAY_CHUNK, length: RELAY_CHUNK },
      { offset: RELAY_CHUNK * 2, length: 1 },
    ])
    expect(chunkPlan(10, 4)).toEqual([
      { offset: 0, length: 4 },
      { offset: 4, length: 4 },
      { offset: 8, length: 2 },
    ])
  })

  it('decodes base64 bytes', () => {
    expect([...decodeBase64(b64(new Uint8Array([0, 1, 254, 255])))]).toEqual([0, 1, 254, 255])
    expect(decodeBase64('').length).toBe(0)
  })

  function worker(object: Uint8Array, ranged: boolean) {
    const calls: Array<Record<string, unknown>> = []
    const host = {
      iii: {
        trigger: async (fn: string, payload: Record<string, unknown>) => {
          expect(fn).toBe('storage::getObject')
          calls.push(payload)
          if (!ranged) {
            return { body_base64: b64(object), content_type: 'video/mp4', size: object.length }
          }
          const offset = Number(payload.offset ?? 0)
          const length = payload.length === undefined ? object.length - offset : Number(payload.length)
          const slice = object.slice(offset, offset + length)
          return {
            body_base64: b64(slice),
            content_type: 'video/mp4',
            size: object.length,
            offset,
            length: slice.length,
          }
        },
      },
    } as unknown as Host
    return { host, calls }
  }

  it('reads an object in ranges on a worker that has them', async () => {
    const object = new Uint8Array(23).map((_, i) => (i * 37) % 256)
    const { host, calls } = worker(object, true)
    const progress: number[] = []
    const blob = await readThroughWorker(host, 'media', 'big.mp4', object.length, {
      chunk: 8,
      onProgress: (done) => progress.push(done),
    })
    expect(calls).toEqual([
      { bucket: 'media', key: 'big.mp4', offset: 0, length: 8 },
      { bucket: 'media', key: 'big.mp4', offset: 8, length: 8 },
      { bucket: 'media', key: 'big.mp4', offset: 16, length: 7 },
    ])
    expect(progress).toEqual([8, 16, 23])
    expect(blob.size).toBe(object.length)
    expect(blob.type).toBe('video/mp4')
    expect([...new Uint8Array(await blob.arrayBuffer())]).toEqual([...object])
  })

  it('takes the whole object from an older worker in one read', async () => {
    const object = new Uint8Array(30).fill(7)
    const { host, calls } = worker(object, false)
    const blob = await readThroughWorker(host, 'media', 'small.mp4', object.length, { chunk: 8 })
    expect(calls).toHaveLength(1)
    expect(blob.size).toBe(object.length)
  })

  it('explains an older worker cannot read past 10 MiB', async () => {
    const host = {
      iii: {
        trigger: async () => {
          // As the published worker sends it: no `message` inside.
          throw {
            code: 'invocation_failed',
            message: 'handler error: {"code":"OBJECT_TOO_LARGE","size":59468103,"cap":10485760}',
          }
        },
      },
    } as unknown as Host
    await expect(readThroughWorker(host, 'media', 'huge.mp4', INLINE_CAP + 1)).rejects.toThrow(RELAY_TOO_OLD)
  })

  it('reads an empty object without a range', async () => {
    const { host, calls } = worker(new Uint8Array(0), true)
    const blob = await readThroughWorker(host, 'media', 'empty', 0, { contentType: 'text/plain' })
    expect(calls).toEqual([{ bucket: 'media', key: 'empty' }])
    expect(blob.size).toBe(0)
    expect(blob.type).toBe('text/plain')
  })

  it('stops when cancelled', async () => {
    const { host } = worker(new Uint8Array(10), true)
    const controller = new AbortController()
    controller.abort()
    await expect(readThroughWorker(host, 'media', 'a', 10, { signal: controller.signal })).rejects.toThrow('Cancelled.')
  })
})
