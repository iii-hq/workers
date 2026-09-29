import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  fetchWorkspaceLayout,
  setWorkspaceLayout,
  WORKSPACE_GET_FUNCTION_ID,
  WORKSPACE_SET_FUNCTION_ID,
} from './workspace-layout'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
const trigger = vi.fn()
beforeEach(() => {
  trigger.mockReset()
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as unknown as Awaited<
    ReturnType<typeof getIiiClient>
  >)
})

describe('workspace layout transport', () => {
  it('talks to the console worker store, never the configuration entry', () => {
    expect(WORKSPACE_GET_FUNCTION_ID).toBe('console::workspace::get')
    expect(WORKSPACE_SET_FUNCTION_ID).toBe('console::workspace::set')
  })

  it('reads and replaces the whole document through console::workspace::*', async () => {
    const stored = {
      tabs: [{ id: 'tab-home', columns: 2, screens: ['chat', 'traces'] }],
      activeTabId: 'tab-home',
    }
    trigger.mockImplementation(async (fn) => {
      if (fn === 'console::workspace::get')
        return { value: stored, path: '/proj/data/ade/workspace.json' }
      return { ok: true }
    })
    expect(await fetchWorkspaceLayout()).toEqual(stored)
    await setWorkspaceLayout({ ...stored, activeTabId: 'tab-2' })
    expect(trigger).toHaveBeenCalledWith('console::workspace::get', {})
    expect(trigger).toHaveBeenCalledWith('console::workspace::set', {
      value: { ...stored, activeTabId: 'tab-2' },
    })
    expect(
      trigger.mock.calls.some(([fn]) =>
        String(fn).startsWith('configuration::'),
      ),
    ).toBe(false)
  })

  it('answers {} for an empty store and null when the worker lacks the function', async () => {
    trigger.mockResolvedValueOnce({ value: {}, path: '' })
    expect(await fetchWorkspaceLayout()).toEqual({})

    trigger.mockRejectedValue(new Error('function_not_found'))
    expect(await fetchWorkspaceLayout()).toBeNull()
    await expect(setWorkspaceLayout({})).rejects.toThrow('function_not_found')
  })

  it('propagates other failures so the writer can retry on the next poll', async () => {
    trigger.mockRejectedValue(new Error('WORKSPACE_UNAVAILABLE: disk full'))
    await expect(fetchWorkspaceLayout()).rejects.toThrow(
      'WORKSPACE_UNAVAILABLE',
    )
  })
})
