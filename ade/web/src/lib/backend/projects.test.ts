import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  deleteHarnessProject,
  HARNESS_PROJECTS_DELETE_FUNCTION_ID,
  HARNESS_PROJECTS_LIST_FUNCTION_ID,
  HARNESS_PROJECTS_UPSERT_FUNCTION_ID,
  listHarnessProjects,
  recentHarnessProjectPaths,
  upsertHarnessProject,
} from './projects'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

describe('harness project catalog', () => {
  const trigger = vi.fn()

  beforeEach(() => {
    trigger.mockReset()
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })

  it('lists projects from the harness backend', async () => {
    trigger.mockResolvedValue({
      projects: [{ path: '/work/harness', name: 'Harness', last_used_at: 42 }],
    })

    await expect(listHarnessProjects()).resolves.toEqual([
      { path: '/work/harness', name: 'Harness', last_used_at: 42 },
    ])
    expect(trigger).toHaveBeenCalledWith(HARNESS_PROJECTS_LIST_FUNCTION_ID, {})
  })

  it('touches without a name and sends explicit renames', async () => {
    trigger.mockResolvedValue({
      project: { path: '/work/harness', name: 'Harness', last_used_at: 42 },
    })

    await upsertHarnessProject('/work/harness')
    await upsertHarnessProject('/work/harness', 'Runtime')

    expect(trigger).toHaveBeenNthCalledWith(
      1,
      HARNESS_PROJECTS_UPSERT_FUNCTION_ID,
      { path: '/work/harness' },
    )
    expect(trigger).toHaveBeenNthCalledWith(
      2,
      HARNESS_PROJECTS_UPSERT_FUNCTION_ID,
      { path: '/work/harness', name: 'Runtime' },
    )
  })

  it('deletes a project by path', async () => {
    trigger.mockResolvedValue({ deleted: true })
    await expect(deleteHarnessProject('/work/harness')).resolves.toBe(true)
    expect(trigger).toHaveBeenCalledWith(HARNESS_PROJECTS_DELETE_FUNCTION_ID, {
      path: '/work/harness',
    })
  })

  it('keeps the synchronous extension view aligned with backend changes', async () => {
    trigger
      .mockResolvedValueOnce({
        projects: [
          { path: '/work/older', name: 'Older', last_used_at: 10 },
          { path: '/work/recent', name: 'Recent', last_used_at: 20 },
        ],
      })
      .mockResolvedValueOnce({
        project: { path: '/work/new', name: 'New', last_used_at: 30 },
      })
      .mockResolvedValueOnce({ deleted: true })

    await listHarnessProjects()
    expect(recentHarnessProjectPaths()).toEqual(['/work/recent', '/work/older'])

    await upsertHarnessProject('/work/new')
    expect(recentHarnessProjectPaths()).toEqual([
      '/work/new',
      '/work/recent',
      '/work/older',
    ])

    await deleteHarnessProject('/work/recent')
    expect(recentHarnessProjectPaths()).toEqual(['/work/new', '/work/older'])
  })
})
