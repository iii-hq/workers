import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  errMsg,
  fetchDefaultWorkingDir,
  HARNESS_FILESYSTEM_INFO_FUNCTION_ID,
  resetDefaultWorkingDirForTests,
  validateWorkspaceDir,
  WORKSPACE_VALIDATE_FUNCTION_ID,
} from './working-dir'

const triggerMock = vi.fn()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger: triggerMock }),
}))

beforeEach(() => {
  triggerMock.mockReset()
  resetDefaultWorkingDirForTests()
})

describe('errMsg', () => {
  it('unwraps the innermost nested handler message', () => {
    const raw =
      'handler error: {"code":"C211","message":"not found or denied: /gone"}'
    expect(errMsg(new Error(raw))).toBe('not found or denied: /gone')
  })

  it('falls back to the raw message when nothing nests', () => {
    expect(errMsg(new Error('plain failure'))).toBe('plain failure')
  })
})

describe('validateWorkspaceDir', () => {
  it('returns the worker-echoed canonical path', async () => {
    triggerMock.mockResolvedValueOnce({ path: '/real/project' })
    const res = await validateWorkspaceDir('/link/project')
    expect(res).toEqual({ ok: true, path: '/real/project' })
    expect(triggerMock).toHaveBeenCalledWith(WORKSPACE_VALIDATE_FUNCTION_ID, {
      path: '/link/project',
    })
  })

  it('maps a rejection to ok:false with the unwrapped message', async () => {
    triggerMock.mockRejectedValueOnce({
      message: 'handler error: {"code":"S212","message":"not a directory"}',
    })
    const res = await validateWorkspaceDir('/gone')
    expect(res).toEqual({ ok: false, error: 'not a directory' })
  })
})

describe('fetchDefaultWorkingDir', () => {
  it('resolves the harness default and stores the shell-canonical path', async () => {
    triggerMock
      .mockResolvedValueOnce({ default_root: '/work/stack' })
      .mockResolvedValueOnce({ path: '/work/stack-canonical' })
    await expect(fetchDefaultWorkingDir()).resolves.toBe(
      '/work/stack-canonical',
    )
    expect(triggerMock).toHaveBeenNthCalledWith(
      1,
      HARNESS_FILESYSTEM_INFO_FUNCTION_ID,
      {},
    )
    expect(triggerMock).toHaveBeenNthCalledWith(
      2,
      WORKSPACE_VALIDATE_FUNCTION_ID,
      { path: '/work/stack' },
    )
  })

  it('caches the resolution for the page lifetime', async () => {
    triggerMock
      .mockResolvedValueOnce({ default_root: '/work/stack' })
      .mockResolvedValueOnce({ path: '/work/stack' })
    await fetchDefaultWorkingDir()
    await fetchDefaultWorkingDir()
    expect(triggerMock).toHaveBeenCalledTimes(2)
  })

  it('is null when the harness reports no default (defaulting off)', async () => {
    triggerMock.mockResolvedValueOnce({ default_root: null })
    await expect(fetchDefaultWorkingDir()).resolves.toBeNull()
    expect(triggerMock).toHaveBeenCalledTimes(1)
  })

  it('is null when the shell rejects the harness default', async () => {
    triggerMock
      .mockResolvedValueOnce({ default_root: '/vm/only/path' })
      .mockRejectedValueOnce({
        message: 'handler error: {"code":"S212","message":"not a directory"}',
      })
    await expect(fetchDefaultWorkingDir()).resolves.toBeNull()
  })

  it('is null when the harness function is missing (older harness)', async () => {
    triggerMock.mockRejectedValueOnce({ message: 'function not found' })
    await expect(fetchDefaultWorkingDir()).resolves.toBeNull()
  })
})
