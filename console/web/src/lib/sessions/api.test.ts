import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { deleteAttachment, fetchTranscript, setSessionDraft } from './api'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

describe('setSessionDraft', () => {
  const trigger = vi.fn()

  beforeEach(() => {
    trigger.mockReset()
    trigger.mockResolvedValue({ draft: null, attachments: [] })
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })

  /* A keystroke save must leave the parked attachments alone: the server
     treats an OMITTED list as "unchanged" and an empty one as "clear". */
  it('omits attachment_ids on a text-only save', async () => {
    await setSessionDraft('s-1', 'half a thought')
    expect(trigger).toHaveBeenCalledWith('session::set-draft', {
      session_id: 's-1',
      draft: 'half a thought',
    })
  })

  it('sends the parked list in chip order when given', async () => {
    await setSessionDraft('s-1', 'see attached', ['a_2', 'a_1'])
    expect(trigger).toHaveBeenCalledWith('session::set-draft', {
      session_id: 's-1',
      draft: 'see attached',
      attachment_ids: ['a_2', 'a_1'],
    })
  })

  /* The post-send clear: no text AND an explicit empty list, so the server
     draft is fully emptied in one write. */
  it('sends an explicit empty list when clearing', async () => {
    await setSessionDraft('s-1', null, [])
    expect(trigger).toHaveBeenCalledWith('session::set-draft', {
      session_id: 's-1',
      attachment_ids: [],
    })
  })

  it('returns what the server now holds', async () => {
    trigger.mockResolvedValue({ draft: 'x', attachments: [] })
    await expect(setSessionDraft('s-1', 'x')).resolves.toEqual({
      draft: 'x',
      attachments: [],
    })
  })
})

describe('deleteAttachment', () => {
  it('releases one stored attachment by id', async () => {
    const trigger = vi.fn().mockResolvedValue({ deleted: true })
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
    await expect(
      deleteAttachment({ session_id: 's-1', attachment_id: 'a_1' }),
    ).resolves.toEqual({ deleted: true })
    expect(trigger).toHaveBeenCalledWith('session::delete-attachment', {
      session_id: 's-1',
      attachment_id: 'a_1',
    })
  })
})

describe('fetchTranscript', () => {
  const trigger = vi.fn()

  beforeEach(() => {
    trigger.mockReset()
    trigger.mockResolvedValue({ messages: [], next_cursor: null })
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })

  /* Opening a session must not pull every past screenshot: the bytes stay in
     the store and the chips fetch them on view. */
  it('leaves image bytes out by default', async () => {
    await fetchTranscript('s-1')
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(trigger.mock.calls[0][0]).toBe('session::messages')
    expect(trigger.mock.calls[0][1]).toMatchObject({
      session_id: 's-1',
      include_custom: true,
      include_image_data: false,
    })
  })

  /* An export or a compaction hands the transcript on, and the pictures
     have to come with it. Omitting the flag (not sending `true`) keeps the
     payload identical to what older workers already accept. */
  it('omits the flag when the caller wants the bytes', async () => {
    await fetchTranscript('s-1', { includeImageData: true })
    expect(trigger.mock.calls[0][1]).not.toHaveProperty('include_image_data')
  })

  it('carries the flag onto every page', async () => {
    trigger
      .mockResolvedValueOnce({
        messages: [{ entry_id: 'e1' }],
        next_cursor: 'c2',
      })
      .mockResolvedValueOnce({
        messages: [{ entry_id: 'e2' }],
        next_cursor: null,
      })
    const items = await fetchTranscript('s-1')
    expect(items.map((i) => i.entry_id)).toEqual(['e1', 'e2'])
    expect(trigger).toHaveBeenCalledTimes(2)
    for (const call of trigger.mock.calls) {
      expect(call[1]).toMatchObject({ include_image_data: false })
    }
    expect(trigger.mock.calls[1][1]).toMatchObject({ cursor: 'c2' })
  })
})
