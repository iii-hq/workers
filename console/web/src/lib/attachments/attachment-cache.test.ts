import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  attachmentCacheKey,
  clearAttachmentCache,
  loadAttachment,
  peekAttachment,
} from './attachment-cache'
import { GET_ATTACHMENT_FUNCTION_ID } from './store'

function stored(attachmentId: string, data = 'AAAA', mime = 'image/png') {
  return {
    attachment: {
      attachment_id: attachmentId,
      session_id: 's-1',
      name: `${attachmentId}.png`,
      mime,
      size: 3,
      sha256: 'deadbeef',
      created_at: 1,
    },
    data,
  }
}

/** A trigger whose answer the test releases by hand. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (err: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

describe('loadAttachment', () => {
  afterEach(() => clearAttachmentCache())

  it('asks the store for the bytes, once, with include_data', async () => {
    const trigger = vi.fn(async () => stored('a_1'))
    const cached = await loadAttachment('s-1', 'a_1', trigger)
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(trigger).toHaveBeenCalledWith(GET_ATTACHMENT_FUNCTION_ID, {
      session_id: 's-1',
      attachment_id: 'a_1',
      include_data: true,
    })
    expect(cached).toMatchObject({
      data: 'AAAA',
      dataUrl: 'data:image/png;base64,AAAA',
      attachment: { name: 'a_1.png' },
    })
  })

  /* Two chips for the same picture mounting in one frame — or the thumbnail
     and the download arrow — must share one request. */
  it('shares one in-flight request between concurrent callers', async () => {
    const gate = deferred<ReturnType<typeof stored>>()
    const trigger = vi.fn(() => gate.promise)
    const first = loadAttachment('s-1', 'a_1', trigger)
    const second = loadAttachment('s-1', 'a_1', trigger)
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(peekAttachment('s-1', 'a_1')).toBeUndefined()
    gate.resolve(stored('a_1'))
    const [a, b] = await Promise.all([first, second])
    expect(a).toBe(b)
    expect(peekAttachment('s-1', 'a_1')).toBe(a)
  })

  /* Scrolling back, switching sessions and returning, a remount after a
     snapshot: none of these may fetch again. */
  it('answers a resolved entry without a new request', async () => {
    const trigger = vi.fn(async () => stored('a_1'))
    await loadAttachment('s-1', 'a_1', trigger)
    const again = await loadAttachment('s-1', 'a_1', trigger)
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(again.dataUrl).toBe('data:image/png;base64,AAAA')
  })

  it('keys by session as well as attachment', async () => {
    const trigger = vi.fn(async () => stored('a_1'))
    await loadAttachment('s-1', 'a_1', trigger)
    await loadAttachment('s-2', 'a_1', trigger)
    expect(trigger).toHaveBeenCalledTimes(2)
    expect(attachmentCacheKey('s-1', 'a_1')).not.toBe(
      attachmentCacheKey('s-2', 'a_1'),
    )
  })

  /* A failure must not be remembered: the chip offers a retry, and a retry
     that replays the cached rejection would never recover. */
  it('forgets a failed fetch so the next call retries', async () => {
    const trigger = vi
      .fn()
      .mockRejectedValueOnce(new Error('session/not_found: gone'))
      .mockResolvedValueOnce(stored('a_1'))
    await expect(loadAttachment('s-1', 'a_1', trigger)).rejects.toThrow('gone')
    expect(peekAttachment('s-1', 'a_1')).toBeUndefined()
    await expect(loadAttachment('s-1', 'a_1', trigger)).resolves.toMatchObject({
      data: 'AAAA',
    })
    expect(trigger).toHaveBeenCalledTimes(2)
  })

  it('treats a store that no longer has the bytes as a failure', async () => {
    const trigger = vi.fn(async () => ({ ...stored('a_1'), data: null }))
    await expect(loadAttachment('s-1', 'a_1', trigger)).rejects.toThrow(
      'no longer in the session store',
    )
    const missing = vi.fn(async () => null)
    await expect(loadAttachment('s-1', 'a_2', missing)).rejects.toThrow()
  })
})
