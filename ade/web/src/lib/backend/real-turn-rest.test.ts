import { beforeEach, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { realBackend } from './real'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

const call = vi.fn()
const ended = { status: 'completed', expects_wake: false, open_asks: 0 }

beforeEach(() => {
  call.mockReset().mockResolvedValue(ended)
  vi.mocked(getIiiClient).mockResolvedValue({ call } as never)
})

it("reads the session's harness status once", async () => {
  expect(await realBackend.turnRest?.('s-1')).toBe('ended')
  expect(call).toHaveBeenCalledExactlyOnceWith('harness::status', {
    session_id: 's-1',
    verbose: true,
  })
})

it('tells a turn that waits', async () => {
  call.mockResolvedValue({ ...ended, expects_wake: true })
  expect(await realBackend.turnRest?.('s-1')).toBe('waiting')
})

it('does not read a failed call as an end', async () => {
  call.mockRejectedValue(new Error('status failed'))
  await expect(realBackend.turnRest?.('s-1')).rejects.toThrow('status failed')
})
