import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { EditorCache } from '../EditorPane'
import { createFilePrefetcher } from '../file-prefetch'

function setup(overrides: Partial<Parameters<typeof createFilePrefetcher>[0]> = {}) {
  const call = vi.fn(async (_fn: string, args: { path: string }) => ({
    content: `body of ${args.path}`,
    revision: 'r1',
    is_utf8: true,
    more_lines: false,
    mode: 420,
    size: 12,
  }))
  const host = { iii: { call } } as unknown as Parameters<typeof createFilePrefetcher>[0]['host']
  const cache: EditorCache = new Map()
  let token = 1
  const prefetcher = createFilePrefetcher({
    host,
    cache,
    resolve: (rel) => (rel.startsWith('secret/') ? null : { abs: `/root/${rel}`, token }),
    isOpen: () => false,
    delayMs: 50,
    keep: 2,
    ...overrides,
  })
  return { call, cache, prefetcher, bump: () => void (token += 1) }
}

describe('createFilePrefetcher', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('reads a file once the pointer has rested on it, into the editor cache', async () => {
    const { call, cache, prefetcher } = setup()
    prefetcher.prefetch('src/a.ts')
    expect(call).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(60)
    expect(call).toHaveBeenCalledTimes(1)
    expect(call.mock.calls[0]?.[1]).toMatchObject({ path: '/root/src/a.ts' })
    expect(cache.get('src/a.ts')).toMatchObject({
      savedContent: 'body of /root/src/a.ts',
      draft: 'body of /root/src/a.ts',
      revision: 'r1',
      readOnly: null,
    })
  })

  it('only reads the row the pointer stopped on, not every row it crossed', async () => {
    const { call, prefetcher } = setup()
    prefetcher.prefetch('a.ts')
    await vi.advanceTimersByTimeAsync(20)
    prefetcher.prefetch('b.ts')
    await vi.advanceTimersByTimeAsync(20)
    prefetcher.prefetch('c.ts')
    await vi.advanceTimersByTimeAsync(60)
    expect(call).toHaveBeenCalledTimes(1)
    expect(call.mock.calls[0]?.[1]).toMatchObject({ path: '/root/c.ts' })
  })

  it('cancels a pending read when the pointer leaves the tree', async () => {
    const { call, prefetcher } = setup()
    prefetcher.prefetch('a.ts')
    prefetcher.cancel()
    await vi.advanceTimersByTimeAsync(100)
    expect(call).not.toHaveBeenCalled()
  })

  it('leaves cached, protected and image files alone', async () => {
    const { call, cache, prefetcher } = setup()
    cache.set('open.ts', { savedContent: 'x', draft: 'x', readOnly: null, mode: null, size: null })
    prefetcher.prefetch('open.ts')
    await vi.advanceTimersByTimeAsync(60)
    prefetcher.prefetch('secret/.env')
    await vi.advanceTimersByTimeAsync(60)
    prefetcher.prefetch('logo.png')
    await vi.advanceTimersByTimeAsync(60)
    expect(call).not.toHaveBeenCalled()
    expect(cache.get('open.ts')?.draft).toBe('x')
  })

  it('does not cache a truncated or binary read, and does not ask again', async () => {
    const { call, cache, prefetcher } = setup()
    call.mockResolvedValueOnce({ content: 'head', revision: 'r', is_utf8: true, more_lines: true, mode: 420, size: 1 })
    prefetcher.prefetch('big.log')
    await vi.advanceTimersByTimeAsync(60)
    expect(cache.has('big.log')).toBe(false)
    prefetcher.prefetch('big.log')
    await vi.advanceTimersByTimeAsync(60)
    expect(call).toHaveBeenCalledTimes(1)
  })

  it('drops a read that lands after the root changed', async () => {
    const { call, cache, prefetcher, bump } = setup()
    let release: (value: Awaited<ReturnType<typeof call>>) => void = () => {}
    call.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve
        }),
    )
    prefetcher.prefetch('a.ts')
    await vi.advanceTimersByTimeAsync(50)
    expect(call).toHaveBeenCalledTimes(1)
    bump()
    release({ content: 'late', revision: 'r', is_utf8: true, more_lines: false, mode: 420, size: 4 })
    await vi.advanceTimersByTimeAsync(10)
    expect(cache.has('a.ts')).toBe(false)
  })

  it('evicts its oldest unopened entries past the keep limit, never an open tab', async () => {
    const open = new Set(['a.ts'])
    const { cache, prefetcher } = setup({ isOpen: (rel) => open.has(rel) })
    for (const rel of ['a.ts', 'b.ts', 'c.ts', 'd.ts']) {
      prefetcher.prefetch(rel)
      await vi.advanceTimersByTimeAsync(60)
    }
    expect(cache.has('a.ts')).toBe(true)
    expect(cache.has('b.ts')).toBe(false)
    expect(cache.has('c.ts')).toBe(true)
    expect(cache.has('d.ts')).toBe(true)
  })
})
