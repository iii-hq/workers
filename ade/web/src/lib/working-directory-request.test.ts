import { describe, expect, it, vi } from 'vitest'

import {
  onWorkingDirectoryChangeRequest,
  requestWorkingDirectoryChange,
} from './working-directory-request'

describe('working-directory requests', () => {
  it('routes a normalized request to the first matching session', () => {
    const ignored = vi.fn(() => false)
    const accepted = vi.fn(() => true)
    const duplicate = vi.fn(() => true)
    const disposeIgnored = onWorkingDirectoryChangeRequest(ignored)
    const disposeAccepted = onWorkingDirectoryChangeRequest(accepted)
    const disposeDuplicate = onWorkingDirectoryChangeRequest(duplicate)

    expect(
      requestWorkingDirectoryChange({
        sessionId: ' session-1 ',
        path: ' /private/tmp/project ',
      }),
    ).toBe(true)
    expect(accepted).toHaveBeenCalledWith({
      sessionId: 'session-1',
      path: '/private/tmp/project',
    })
    expect(duplicate).not.toHaveBeenCalled()

    disposeIgnored()
    disposeAccepted()
    disposeDuplicate()
  })

  it('rejects incomplete requests and removes listeners', () => {
    const listener = vi.fn(() => true)
    const dispose = onWorkingDirectoryChangeRequest(listener)

    expect(
      requestWorkingDirectoryChange({ sessionId: '', path: '/tmp/project' }),
    ).toBe(false)
    dispose()
    expect(
      requestWorkingDirectoryChange({
        sessionId: 'session-1',
        path: '/tmp/project',
      }),
    ).toBe(false)
    expect(listener).not.toHaveBeenCalled()
  })
})
