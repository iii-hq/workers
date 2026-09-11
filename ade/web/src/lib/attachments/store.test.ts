import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  base64ToBlob,
  type FileBlock,
  hasStorableAttachments,
  linkStoredAttachments,
  PUT_ATTACHMENT_FUNCTION_ID,
  uploadAttachments,
} from './store'

function picked(
  name: string,
  bytes = 'hello',
  type = 'text/plain',
): Attachment {
  return {
    id: `chip-${name}`,
    name,
    size: bytes.length,
    type,
    file: new File([bytes], name, { type }),
  }
}

function stored(name: string, mime: string, size: number) {
  const block: FileBlock = {
    type: 'file',
    attachment_id: `a_${name}`,
    name,
    mime,
    size,
  }
  return {
    attachment: {
      attachment_id: block.attachment_id,
      session_id: 's-1',
      name,
      mime,
      size,
      sha256: 'deadbeef',
      created_at: 1,
    },
    block,
  }
}

describe('uploadAttachments', () => {
  it('stores every attachment with bytes and returns its file block', async () => {
    const trigger = vi.fn(
      async (_fn: string, payload: Record<string, unknown>) =>
        stored(
          payload.name as string,
          payload.mime as string,
          (payload.data as string).length,
        ),
    )
    const result = await uploadAttachments(
      's-1',
      [picked('notes.txt'), picked('shot.png', '\x89PNG', 'image/png')],
      trigger,
    )

    expect(trigger).toHaveBeenCalledTimes(2)
    expect(trigger.mock.calls[0][0]).toBe(PUT_ATTACHMENT_FUNCTION_ID)
    expect(trigger.mock.calls[0][1]).toEqual({
      session_id: 's-1',
      name: 'notes.txt',
      mime: 'text/plain',
      data: btoa('hello'),
    })
    expect(result.blocks.map((b) => [b.type, b.name, b.mime])).toEqual([
      ['file', 'notes.txt', 'text/plain'],
      ['file', 'shot.png', 'image/png'],
    ])
    expect(result.uploaded).toEqual([
      { id: 'chip-notes.txt', attachmentId: 'a_notes.txt' },
      { id: 'chip-shot.png', attachmentId: 'a_shot.png' },
    ])
    expect(result.failures).toEqual([])
  })

  /* A file dragged out of an archive often has no type at all; the store
     requires one, and the generic binary type is what it gets. */
  it('falls back to application/octet-stream for a typeless file', async () => {
    const trigger = vi.fn(
      async (_fn: string, payload: Record<string, unknown>) =>
        stored(payload.name as string, payload.mime as string, 3),
    )
    await uploadAttachments('s-1', [picked('blob.bin', 'abc', '')], trigger)
    expect(trigger.mock.calls[0][1]).toMatchObject({
      mime: 'application/octet-stream',
    })
  })

  /* Nothing on the send path throws: a store that is down turns into a
     notice, and the message still goes with its expansions. */
  it('turns a rejected upload into a failure and keeps going', async () => {
    const trigger = vi.fn(
      async (_fn: string, payload: Record<string, unknown>) => {
        if (payload.name === 'big.pdf') {
          throw { message: 'session/attachment_too_large: 90 MB > 64 MB' }
        }
        return stored(payload.name as string, payload.mime as string, 5)
      },
    )
    const result = await uploadAttachments(
      's-1',
      [picked('big.pdf', 'x', 'application/pdf'), picked('ok.txt')],
      trigger,
    )
    expect(result.failures).toEqual([
      { name: 'big.pdf', reason: 'larger than the attachment store accepts' },
    ])
    expect(result.blocks.map((b) => b.name)).toEqual(['ok.txt'])
    expect(result.uploaded).toEqual([
      { id: 'chip-ok.txt', attachmentId: 'a_ok.txt' },
    ])
  })

  it('treats an answer without a block as a failure', async () => {
    const trigger = vi.fn(async () => ({}))
    const result = await uploadAttachments('s-1', [picked('a.txt')], trigger)
    expect(result.blocks).toEqual([])
    expect(result.failures).toHaveLength(1)
    expect(result.failures[0].name).toBe('a.txt')
  })

  /* A conversation reloaded from history carries chips, not bytes; those
     were stored when they were sent. */
  it('skips attachments without bytes', async () => {
    const trigger = vi.fn()
    const result = await uploadAttachments(
      's-1',
      [{ id: 'old', name: 'old.pdf', size: 9, type: 'application/pdf' }],
      trigger,
    )
    expect(trigger).not.toHaveBeenCalled()
    expect(result).toEqual({ blocks: [], uploaded: [], failures: [] })
  })

  /* A draft chip is stored while the message is being composed (and a
     restored one was stored before the refresh). Sending it must reference
     that copy, not upload a second one. */
  it('reuses already-stored chips instead of uploading them again', async () => {
    const trigger = vi.fn(
      async (_fn: string, payload: Record<string, unknown>) =>
        stored(payload.name as string, payload.mime as string, 5),
    )
    const result = await uploadAttachments(
      's-1',
      [
        {
          id: 'parked',
          name: 'kept.pdf',
          size: 9,
          type: 'application/pdf',
          attachmentId: 'a_kept',
        },
        picked('fresh.txt'),
        {
          id: 'typeless',
          name: 'blob',
          size: 2,
          type: '',
          attachmentId: 'a_blob',
          file: new File(['ab'], 'blob'),
        },
      ],
      trigger,
    )
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(trigger.mock.calls[0][1]).toMatchObject({ name: 'fresh.txt' })
    expect(result.blocks).toEqual([
      {
        type: 'file',
        attachment_id: 'a_kept',
        name: 'kept.pdf',
        mime: 'application/pdf',
        size: 9,
      },
      expect.objectContaining({ type: 'file', name: 'fresh.txt' }),
      {
        type: 'file',
        attachment_id: 'a_blob',
        name: 'blob',
        mime: 'application/octet-stream',
        size: 2,
      },
    ])
    expect(result.uploaded).toEqual([
      { id: 'parked', attachmentId: 'a_kept' },
      { id: 'chip-fresh.txt', attachmentId: 'a_fresh.txt' },
      { id: 'typeless', attachmentId: 'a_blob' },
    ])
    expect(result.failures).toEqual([])
  })
})

describe('linkStoredAttachments', () => {
  /* The expansion reads `attachmentId` off the chip to stamp the image
     block; the upload that produces the id runs first, and this is the join.
     A chip the store refused stays as it was. */
  it('puts the stored ids on their chips and leaves the rest alone', () => {
    const shot = picked('shot.png', '\x89PNG', 'image/png')
    const notes = picked('notes.txt')
    const linked = linkStoredAttachments(
      [shot, notes],
      [{ id: shot.id, attachmentId: 'a_shot' }],
    )
    expect(linked[0]).toMatchObject({ id: shot.id, attachmentId: 'a_shot' })
    expect(linked[0].file).toBe(shot.file)
    expect(linked[1]).toBe(notes)
  })

  it('returns the same list when nothing was stored', () => {
    const chips = [picked('a.txt')]
    expect(linkStoredAttachments(chips, [])).toBe(chips)
  })
})

describe('hasStorableAttachments', () => {
  it('is true for bytes or a stored reference, false for a bare chip', () => {
    expect(hasStorableAttachments([picked('a.txt')])).toBe(true)
    expect(
      hasStorableAttachments([
        {
          id: 'r',
          name: 'r.pdf',
          size: 1,
          type: 'application/pdf',
          attachmentId: 'a_r',
        },
      ]),
    ).toBe(true)
    expect(
      hasStorableAttachments([
        { id: 'h', name: 'h.pdf', size: 1, type: 'application/pdf' },
      ]),
    ).toBe(false)
  })
})

describe('base64ToBlob', () => {
  it('restores the exact bytes and the declared type', async () => {
    const blob = base64ToBlob(btoa('\x00\x01\xffabc'), 'application/pdf')
    expect(blob.type).toBe('application/pdf')
    expect([...new Uint8Array(await blob.arrayBuffer())]).toEqual([
      0, 1, 255, 97, 98, 99,
    ])
  })
})
