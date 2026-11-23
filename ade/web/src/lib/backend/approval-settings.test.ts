import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { approveAlways } from './approval-settings'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

const call = vi.fn()

beforeEach(() => {
  call.mockReset()
  vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
})

describe('approveAlways', () => {
  it('sends the namespace and keeps it on returned entries', async () => {
    call.mockResolvedValue({
      settings: {
        approved_always: [
          {
            namespace: 'laptop',
            function_id: 'shell::exec',
            granted_at: 1,
            granted_by: 'user_click',
          },
        ],
      },
    })
    const s = await approveAlways('s1', 'shell::exec', 'laptop')
    expect(call).toHaveBeenCalledWith('approval::approve-always', {
      session_id: 's1',
      function_id: 'shell::exec',
      namespace: 'laptop',
    })
    expect(s.approved_always[0].namespace).toBe('laptop')
  })

  it('omits namespace when the call has none', async () => {
    call.mockResolvedValue({ settings: {} })
    await approveAlways('s1', 'shell::exec')
    expect(call).toHaveBeenCalledWith('approval::approve-always', {
      session_id: 's1',
      function_id: 'shell::exec',
    })
  })
})
