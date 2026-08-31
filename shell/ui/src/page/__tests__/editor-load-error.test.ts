import { describe, expect, it } from 'vitest'
import { loadErrorMessage } from '../load-error'

describe('loadErrorMessage', () => {
  it('turns a missing-file handler error into plain words', () => {
    const raw =
      'handler error: {"code":"C211","message":"/w/demo-b.txt: not found or not accessible. Verify the path with coder::list-folder or coder::tree."}'
    expect(loadErrorMessage(raw)).toBe(
      'this file no longer exists on disk: deleted or moved after this tab was opened',
    )
  })

  it('unwraps other handler errors to their message', () => {
    const raw = 'handler error: {"code":"C400","message":"permission denied: /etc/hosts"}'
    expect(loadErrorMessage(raw)).toBe('permission denied: /etc/hosts')
  })

  it('leaves plain text alone', () => {
    expect(loadErrorMessage('network unreachable')).toBe('network unreachable')
  })
})
