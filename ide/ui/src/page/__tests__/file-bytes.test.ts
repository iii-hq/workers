import { describe, expect, it, vi } from 'vitest'
import { chunkRanges, decodeBase64, readFileBytes } from '../file-bytes'

describe('file-bytes', () => {
  it('splits a size into ranges', () => {
    expect(chunkRanges(10, 4)).toEqual([
      { offset: 0, length: 4 },
      { offset: 4, length: 4 },
      { offset: 8, length: 2 },
    ])
    expect(chunkRanges(0, 4)).toEqual([])
  })

  it('decodes base64 to bytes', () => {
    expect([...decodeBase64(btoa('hi!'))]).toEqual([104, 105, 33])
  })

  it('streams chunks in order into one blob and refuses oversized files', async () => {
    const body = new Uint8Array(10).map((_, i) => i)
    const encode = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes))
    const trigger = vi.fn(async (_fn: string, payload: { offset: number; length: number }) => {
      const slice = body.slice(payload.offset, payload.offset + payload.length)
      return {
        path: '/r/x.png',
        size: body.length,
        offset: payload.offset,
        length: slice.length,
        content: encode(slice),
        mtime: 5,
        eof: payload.offset + slice.length >= body.length,
      }
    })
    const host = { iii: { trigger } } as unknown as Parameters<typeof readFileBytes>[0]
    const progress: number[] = []
    const out = await readFileBytes(host, '/r/x.png', 'image/png', {
      chunkBytes: 4,
      parallel: 2,
      onProgress: (received) => progress.push(received),
    })
    expect(out.size).toBe(10)
    expect(out.mtime).toBe(5)
    expect(new Uint8Array(await out.blob.arrayBuffer())).toEqual(body)
    expect(progress.at(-1)).toBe(10)
    expect(trigger).toHaveBeenCalledTimes(3)
    await expect(readFileBytes(host, '/r/x.png', 'image/png', { chunkBytes: 4, maxBytes: 5 })).rejects.toThrow(
      'preview limit',
    )
  })
})
