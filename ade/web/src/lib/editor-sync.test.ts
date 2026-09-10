import { beforeEach, describe, expect, it, vi } from 'vitest'

import { resetEditorSyncForTests, syncEditorWorkspace } from './editor-sync'

describe('syncEditorWorkspace', () => {
  beforeEach(() => {
    resetEditorSyncForTests()
  })

  it('opens the editor workspace at the given root', async () => {
    const trigger = vi.fn().mockResolvedValue({})
    await expect(syncEditorWorkspace('/repo', trigger)).resolves.toBe(true)
    expect(trigger).toHaveBeenCalledWith('editor::workspace::open', {
      root: '/repo',
    })
  })

  it('dedupes consecutive calls for the same root', async () => {
    const trigger = vi.fn().mockResolvedValue({})
    await syncEditorWorkspace('/repo', trigger)
    await syncEditorWorkspace('/repo', trigger)
    expect(trigger).toHaveBeenCalledTimes(1)
    await syncEditorWorkspace('/other', trigger)
    expect(trigger).toHaveBeenCalledTimes(2)
  })

  it('swallows failures and retries on the next call', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('function_not_found'))
    await expect(syncEditorWorkspace('/repo', trigger)).resolves.toBe(false)
    // The failed root was not recorded, so the same root retries.
    await syncEditorWorkspace('/repo', trigger)
    expect(trigger).toHaveBeenCalledTimes(2)
  })

  it('ignores empty roots', async () => {
    const trigger = vi.fn()
    await expect(syncEditorWorkspace('', trigger)).resolves.toBe(false)
    expect(trigger).not.toHaveBeenCalled()
  })
})
