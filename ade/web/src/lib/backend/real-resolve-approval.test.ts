import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { realBackend } from './real'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

describe('approval::resolve request', () => {
  const call = vi.fn()
  const resolve = (
    ...args: Parameters<NonNullable<typeof realBackend.resolveApproval>>
  ) => {
    if (!realBackend.resolveApproval) throw new Error('not registered')
    return realBackend.resolveApproval(...args)
  }
  beforeEach(() => {
    call.mockReset().mockResolvedValue({})
    vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
  })

  it('sends the namespace when given', async () => {
    await resolve('s', 'c', 'allow', { namespace: 'laptop' })
    expect(call).toHaveBeenCalledExactlyOnceWith('approval::resolve', {
      session_id: 's',
      function_call_id: 'c',
      decision: 'allow',
      namespace: 'laptop',
    })
  })

  it('omits the namespace when undefined', async () => {
    await resolve('s', 'c', 'deny', { namespace: undefined })
    await resolve('s', 'c', 'deny')
    for (const [, body] of call.mock.calls) {
      expect(body).not.toHaveProperty('namespace')
    }
  })
})
