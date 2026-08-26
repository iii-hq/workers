import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  activateWorkingDir,
  errMsg,
  fetchDefaultWorkingDir,
  HARNESS_FILESYSTEM_INFO_FUNCTION_ID,
  resetDefaultWorkingDirForTests,
  validateWorkspaceDir,
  WORKSPACE_VALIDATE_FUNCTION_ID,
  workingDirRecoveryNotice,
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
    expect(res).toEqual({ ok: false, error: 'not a directory', code: 'S212' })
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

describe('activateWorkingDir', () => {
  it('keeps a valid saved directory and its canonical path', async () => {
    triggerMock.mockResolvedValueOnce({ path: '/real/project' })

    await expect(activateWorkingDir('/link/project')).resolves.toEqual({
      status: 'valid',
      path: '/real/project',
    })
    expect(triggerMock).toHaveBeenCalledTimes(1)
  })

  it('recovers a deleted saved directory to the live Harness default', async () => {
    triggerMock
      .mockRejectedValueOnce({ code: 'S211', message: 'not found' })
      .mockResolvedValueOnce({ default_root: '/work/stack' })
      .mockResolvedValueOnce({ path: '/work/stack-canonical' })

    await expect(activateWorkingDir('/private/tmp/deleted')).resolves.toEqual({
      status: 'recovered',
      path: '/work/stack-canonical',
    })
  })

  it('clears the current scope when neither saved nor default path is usable', async () => {
    triggerMock
      .mockRejectedValueOnce({ code: 'S211', message: 'not found' })
      .mockResolvedValueOnce({ default_root: null })

    await expect(activateWorkingDir('/private/tmp/deleted')).resolves.toEqual({
      status: 'recovered',
      path: null,
    })
    await expect(fetchDefaultWorkingDir()).resolves.toBeNull()
    expect(triggerMock).toHaveBeenCalledTimes(2)
  })

  it('keeps the saved scope during a transient worker failure', async () => {
    triggerMock.mockRejectedValueOnce({ message: 'worker disconnected' })

    await expect(activateWorkingDir('/work/project')).resolves.toEqual({
      status: 'unavailable',
      path: '/work/project',
    })
    expect(triggerMock).toHaveBeenCalledTimes(1)
  })

  it('does not mistake a missing validation function for a missing directory', async () => {
    triggerMock.mockRejectedValueOnce({ message: 'function not found' })

    await expect(activateWorkingDir('/work/project')).resolves.toEqual({
      status: 'unavailable',
      path: '/work/project',
    })
    expect(triggerMock).toHaveBeenCalledTimes(1)
  })

  it('keeps the saved scope when default resolution is temporarily unavailable', async () => {
    triggerMock
      .mockRejectedValueOnce({ code: 'S211', message: 'not found' })
      .mockRejectedValueOnce({ message: 'harness disconnected' })

    await expect(activateWorkingDir('/private/tmp/deleted')).resolves.toEqual({
      status: 'unavailable',
      path: '/private/tmp/deleted',
    })
  })

  it('does not reuse a stale cached default during recovery', async () => {
    triggerMock
      .mockResolvedValueOnce({ default_root: '/old/default' })
      .mockResolvedValueOnce({ path: '/old/default' })
    await expect(fetchDefaultWorkingDir()).resolves.toBe('/old/default')

    triggerMock
      .mockRejectedValueOnce({
        code: 'S211',
        message: 'saved path was deleted',
      })
      .mockResolvedValueOnce({ default_root: '/new/default' })
      .mockResolvedValueOnce({ path: '/new/default' })

    await expect(activateWorkingDir('/old/default')).resolves.toEqual({
      status: 'recovered',
      path: '/new/default',
    })
    await expect(fetchDefaultWorkingDir()).resolves.toBe('/new/default')
    expect(triggerMock).toHaveBeenCalledTimes(5)
  })
})

describe('workingDirRecoveryNotice', () => {
  it('records fallback and unscoped recovery without presenting an error', () => {
    expect(
      workingDirRecoveryNotice('/private/tmp/deleted', '/work/current'),
    ).toBe(
      'working directory changed to /work/current because /private/tmp/deleted is no longer available — applies to the messages that follow',
    )
    expect(workingDirRecoveryNotice('/private/tmp/deleted', null)).toBe(
      'working directory /private/tmp/deleted is no longer available; this session is now unscoped — applies to the messages that follow',
    )
  })
})
