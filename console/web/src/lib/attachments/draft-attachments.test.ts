import { describe, expect, it, vi } from 'vitest'

import type { Attachment } from '@/types/chat'
import {
  createDraftAttachmentStore,
  draftAttachmentIds,
  draftAttachmentsFromMeta,
  mergeSyncedAttachments,
  reconcileDraftAttachments,
  releaseRemovedDraftAttachments,
  removedStoredAttachmentIds,
} from './draft-attachments'

function chip(id: string, extra: Partial<Attachment> = {}): Attachment {
  return { id, name: `${id}.png`, size: 3, type: 'image/png', ...extra }
}

function picked(id: string, extra: Partial<Attachment> = {}): Attachment {
  return chip(id, {
    file: new File(['abc'], `${id}.png`, { type: 'image/png' }),
    dataUrl: `data:image/png;base64,${id}`,
    ...extra,
  })
}

describe('createDraftAttachmentStore', () => {
  /* The reported bug: switch away and back, and the chips are gone. The store
     hands back the very objects it was given, `File` and thumbnail included. */
  it("round-trips a conversation's chips by id without leaking across ids", () => {
    const store = createDraftAttachmentStore()
    const shot = picked('shot')
    const notes = picked('notes', { type: 'text/plain' })
    store.set('console-a', [shot])
    store.set('console-b', [notes])

    expect(store.get('console-a')).toEqual([shot])
    expect(store.get('console-a')?.[0].file).toBe(shot.file)
    expect(store.get('console-a')?.[0].dataUrl).toBe(shot.dataUrl)
    expect(store.get('console-b')).toEqual([notes])
    expect(store.get('console-c')).toBeUndefined()

    // An emptied list is a real answer (the chips went out on a send), not
    // "unknown".
    store.set('console-a', [])
    expect(store.get('console-a')).toEqual([])
    expect(store.get('console-b')).toEqual([notes])

    store.delete('console-b')
    expect(store.get('console-b')).toBeUndefined()
  })

  /* The composer reports its own state, which lags behind what the store
     learned from an upload; a re-set must not forget the server id. */
  it('carries a learned attachmentId onto a re-reported chip', () => {
    const store = createDraftAttachmentStore()
    store.set('c', [picked('shot')])
    expect(
      store.patch('c', 'shot', { attachmentId: 'a_1' })?.[0],
    ).toMatchObject({
      attachmentId: 'a_1',
    })
    // The composer reports the chip again (a second one attached) still
    // without the id it has not received yet.
    const kept = store.set('c', [picked('shot'), picked('doc')])
    expect(kept.map((a) => a.attachmentId)).toEqual(['a_1', undefined])
    // A chip that left the composer cannot be patched.
    expect(store.patch('c', 'gone', { attachmentId: 'a_x' })).toBeUndefined()
  })
})

describe('draftAttachmentsFromMeta', () => {
  it('turns parked attachments into chips that know their server id', () => {
    expect(
      draftAttachmentsFromMeta({
        draft_attachments: [
          {
            attachment_id: 'a_1',
            session_id: 's',
            name: 'shot.png',
            mime: 'image/png',
            size: 12,
            sha256: 'x',
            created_at: 1,
          },
        ],
      }),
    ).toEqual([
      {
        id: 'a_1',
        name: 'shot.png',
        size: 12,
        type: 'image/png',
        attachmentId: 'a_1',
      },
    ])
    expect(draftAttachmentsFromMeta({})).toBeUndefined()
    expect(draftAttachmentsFromMeta({ draft_attachments: [] })).toBeUndefined()
  })
})

describe('reconcileDraftAttachments', () => {
  it('keeps the hydrated chip objects for ids the server still lists', () => {
    const hydrated = picked('local-1', { attachmentId: 'a_1' })
    const next = reconcileDraftAttachments(
      [hydrated, picked('local-2', { attachmentId: 'a_2' })],
      [
        chip('a_3', { attachmentId: 'a_3' }),
        chip('a_1', { attachmentId: 'a_1' }),
      ],
    )
    // Server order and membership; the known chip is the SAME object.
    expect(next?.map((a) => a.attachmentId)).toEqual(['a_3', 'a_1'])
    expect(next?.[1]).toBe(hydrated)
    expect(reconcileDraftAttachments([hydrated], undefined)).toBeUndefined()
  })
})

describe('removedStoredAttachmentIds / releaseRemovedDraftAttachments', () => {
  const stored = chip('shot', { attachmentId: 'a_shot' })
  const pending = picked('doc')

  it('names only stored chips the user removed', () => {
    expect(
      removedStoredAttachmentIds([stored, pending], [pending], 'remove'),
    ).toEqual(['a_shot'])
    // A chip without a server id has nothing to release.
    expect(
      removedStoredAttachmentIds([stored, pending], [stored], 'remove'),
    ).toEqual([])
  })

  /* Sending clears the composer too — but those chips now ride on a message,
     and their bytes must stay. */
  it('deletes a user-removed uploaded chip but not a send-cleared one', async () => {
    const remove = vi.fn(async () => ({ deleted: true }))

    await expect(
      releaseRemovedDraftAttachments(
        's-1',
        [stored, pending],
        [pending],
        'remove',
        remove,
      ),
    ).resolves.toEqual(['a_shot'])
    expect(remove).toHaveBeenCalledTimes(1)
    expect(remove).toHaveBeenCalledWith({
      session_id: 's-1',
      attachment_id: 'a_shot',
    })

    remove.mockClear()
    await expect(
      releaseRemovedDraftAttachments(
        's-1',
        [stored, pending],
        [],
        'submit',
        remove,
      ),
    ).resolves.toEqual([])
    await releaseRemovedDraftAttachments('s-1', [stored], [], 'edit', remove)
    await releaseRemovedDraftAttachments(
      's-1',
      [stored],
      [stored],
      'hydrate',
      remove,
    )
    expect(remove).not.toHaveBeenCalled()
  })

  it('swallows a failed delete (an orphan is a leak, not a broken draft)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const remove = vi.fn(async () => {
      throw new Error('session/not_found: gone')
    })
    await expect(
      releaseRemovedDraftAttachments('s-1', [stored], [], 'remove', remove),
    ).resolves.toEqual(['a_shot'])
    warn.mockRestore()
  })
})

describe('draftAttachmentIds', () => {
  it('lists stored ids in chip order, skipping chips still uploading', () => {
    expect(
      draftAttachmentIds([
        chip('a', { attachmentId: 'a_a' }),
        picked('b'),
        chip('c', { attachmentId: 'a_c' }),
      ]),
    ).toEqual(['a_a', 'a_c'])
  })
})

describe('mergeSyncedAttachments', () => {
  it('folds learned ids and bytes into matching chips only', () => {
    const current = [picked('shot'), chip('a_2', { attachmentId: 'a_2' })]
    const file = new File(['x'], 'a_2.png', { type: 'image/png' })
    const next = mergeSyncedAttachments(current, [
      chip('shot', { attachmentId: 'a_1' }),
      chip('a_2', { attachmentId: 'a_2', file, dataUrl: 'data:thumb' }),
      picked('never-attached'),
    ])
    expect(next).toHaveLength(2)
    expect(next[0]).toMatchObject({ id: 'shot', attachmentId: 'a_1' })
    expect(next[0].file).toBe(current[0].file)
    expect(next[1]).toMatchObject({ id: 'a_2', file, dataUrl: 'data:thumb' })
  })

  it('returns the same list when nothing is new', () => {
    const current = [chip('a', { attachmentId: 'a_a' })]
    expect(
      mergeSyncedAttachments(current, [chip('a', { attachmentId: 'a_a' })]),
    ).toBe(current)
    expect(mergeSyncedAttachments(current, [])).toBe(current)
    expect(mergeSyncedAttachments([], [chip('a')])).toEqual([])
  })
})
