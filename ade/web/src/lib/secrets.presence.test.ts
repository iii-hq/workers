import { beforeEach, describe, expect, it, vi } from 'vitest'
import { functionRegistered } from '@/lib/function-presence'
import { getIiiClient } from '@/lib/iii-client'
import { detectKeys, getSecret, getSecretsStatus } from './secrets'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('@/lib/function-presence', () => ({ functionRegistered: vi.fn() }))
const call = vi.fn()
beforeEach(() => {
  call.mockReset()
  vi.mocked(getIiiClient).mockResolvedValue({ call } as unknown as Awaited<
    ReturnType<typeof getIiiClient>
  >)
})

describe('secrets helpers without a secrets worker', () => {
  it('answer "not running" without calling the missing functions', async () => {
    vi.mocked(functionRegistered).mockResolvedValue(false)
    expect(await getSecretsStatus()).toBeUndefined()
    expect(await detectKeys(['OPENAI_API_KEY'])).toBeNull()
    expect(await getSecret('OPENAI_API_KEY')).toBeUndefined()
    // A call to an unregistered function is an engine error every time.
    expect(call).not.toHaveBeenCalled()
  })

  it('call the worker when it runs', async () => {
    vi.mocked(functionRegistered).mockResolvedValue(true)
    call.mockResolvedValue({ results: [] })
    expect(await detectKeys(['A'])).toEqual([])
    expect(call).toHaveBeenCalledWith(
      'secrets::detect',
      { names: ['A'] },
      {
        timeoutMs: 15_000,
      },
    )
  })
})
