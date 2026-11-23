import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { getIiiClient } from '@/lib/iii-client'
import {
  deleteSavedFolder,
  HARNESS_FOLDERS_DELETE_FUNCTION_ID,
  HARNESS_FOLDERS_LIST_FUNCTION_ID,
  HARNESS_FOLDERS_UPSERT_FUNCTION_ID,
  LEGACY_FOLDERS_DELETE_FUNCTION_ID,
  LEGACY_FOLDERS_LIST_FUNCTION_ID,
  LEGACY_FOLDERS_UPSERT_FUNCTION_ID,
  listSavedFolders,
  recentFolderPaths,
  upsertSavedFolder,
} from './folders'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

describe('saved folder catalogue', () => {
  const call = vi.fn()

  beforeEach(() => {
    call.mockReset()
    vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
  })
  afterEach(() => setChatCapabilitiesForTests(null))

  describe('on an engine with harness::folders', () => {
    beforeEach(() => setChatCapabilitiesForTests({ folders: true }))

    it('lists folders from harness::folders::list', async () => {
      call.mockResolvedValue({
        folders: [{ path: '/work/harness', name: 'Harness', last_used_at: 42 }],
      })
      await expect(listSavedFolders()).resolves.toEqual([
        { path: '/work/harness', name: 'Harness', last_used_at: 42 },
      ])
      expect(call).toHaveBeenCalledWith(HARNESS_FOLDERS_LIST_FUNCTION_ID, {})
    })

    it('touches without a name and sends explicit renames', async () => {
      call.mockResolvedValue({
        project: { path: '/work/harness', name: 'Harness', last_used_at: 42 },
      })
      await upsertSavedFolder('/work/harness')
      await upsertSavedFolder('/work/harness', 'Runtime')
      expect(call).toHaveBeenNthCalledWith(
        1,
        HARNESS_FOLDERS_UPSERT_FUNCTION_ID,
        { path: '/work/harness' },
      )
      expect(call).toHaveBeenNthCalledWith(
        2,
        HARNESS_FOLDERS_UPSERT_FUNCTION_ID,
        { path: '/work/harness', name: 'Runtime' },
      )
    })

    it('deletes a folder by path', async () => {
      call.mockResolvedValue({ deleted: true })
      await expect(deleteSavedFolder('/work/harness')).resolves.toBe(true)
      expect(call).toHaveBeenCalledWith(HARNESS_FOLDERS_DELETE_FUNCTION_ID, {
        path: '/work/harness',
      })
    })

    it('falls back to the old ids when the new one is not registered', async () => {
      call
        .mockRejectedValueOnce({ code: 'function_not_found', message: 'nope' })
        .mockResolvedValueOnce({
          projects: [{ path: '/work/a', name: 'a', last_used_at: 1 }],
        })
      await expect(listSavedFolders()).resolves.toEqual([
        { path: '/work/a', name: 'a', last_used_at: 1 },
      ])
      expect(call).toHaveBeenLastCalledWith(LEGACY_FOLDERS_LIST_FUNCTION_ID, {})
    })
  })

  describe('on an older harness', () => {
    beforeEach(() => setChatCapabilitiesForTests({ folders: false }))

    it('calls the pre-D25 harness::projects ids', async () => {
      call
        .mockResolvedValueOnce({
          projects: [{ path: '/work/a', name: 'a', last_used_at: 1 }],
        })
        .mockResolvedValueOnce({
          project: { path: '/work/b', name: 'b', last_used_at: 2 },
        })
        .mockResolvedValueOnce({ deleted: true })
      await listSavedFolders()
      await upsertSavedFolder('/work/b')
      await deleteSavedFolder('/work/b')
      expect(call.mock.calls.map(([id]) => id)).toEqual([
        LEGACY_FOLDERS_LIST_FUNCTION_ID,
        LEGACY_FOLDERS_UPSERT_FUNCTION_ID,
        LEGACY_FOLDERS_DELETE_FUNCTION_ID,
      ])
    })
  })

  it('keeps the synchronous extension view aligned with backend changes', async () => {
    setChatCapabilitiesForTests({ folders: true })
    call
      .mockResolvedValueOnce({
        folders: [
          { path: '/work/older', name: 'Older', last_used_at: 10 },
          { path: '/work/recent', name: 'Recent', last_used_at: 20 },
        ],
      })
      .mockResolvedValueOnce({
        project: { path: '/work/new', name: 'New', last_used_at: 30 },
      })
      .mockResolvedValueOnce({ deleted: true })

    await listSavedFolders()
    expect(recentFolderPaths()).toEqual(['/work/recent', '/work/older'])

    await upsertSavedFolder('/work/new')
    expect(recentFolderPaths()).toEqual([
      '/work/new',
      '/work/recent',
      '/work/older',
    ])

    await deleteSavedFolder('/work/recent')
    expect(recentFolderPaths()).toEqual(['/work/new', '/work/older'])
  })
})
