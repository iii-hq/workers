import { beforeEach, describe, expect, it, vi } from 'vitest'

import { resetEditorSyncForTests, syncEditorWorkspace } from './editor-sync'

describe('syncEditorWorkspace', () => {
  beforeEach(() => {
    resetEditorSyncForTests()
  })

  it('opens the editor workspace at the given root', async () => {
    const call = vi.fn().mockResolvedValue({})
    await expect(syncEditorWorkspace('/repo', call)).resolves.toBe(true)
    expect(call).toHaveBeenCalledWith('editor::workspace::open', {
      root: '/repo',
    })
  })

  it('dedupes consecutive calls for the same root', async () => {
    const call = vi.fn().mockResolvedValue({})
    await syncEditorWorkspace('/repo', call)
    await syncEditorWorkspace('/repo', call)
    expect(call).toHaveBeenCalledTimes(1)
    await syncEditorWorkspace('/other', call)
    expect(call).toHaveBeenCalledTimes(2)
  })

  it('swallows failures and retries on the next call', async () => {
    const call = vi.fn().mockRejectedValue(new Error('function_not_found'))
    await expect(syncEditorWorkspace('/repo', call)).resolves.toBe(false)
    // The failed root was not recorded, so the same root retries.
    await syncEditorWorkspace('/repo', call)
    expect(call).toHaveBeenCalledTimes(2)
  })

  it('ignores empty roots', async () => {
    const call = vi.fn()
    await expect(syncEditorWorkspace('', call)).resolves.toBe(false)
    expect(call).not.toHaveBeenCalled()
  })
})
