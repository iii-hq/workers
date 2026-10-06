import { beforeEach, describe, expect, it, vi } from 'vitest'
import { runStep } from './api'
import type { PlanStep } from './plan'

const trigger = vi.fn()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger }),
}))

const META = {
  name: 'DEEPSEEK_API_KEY',
  hint: 'sk-e2e…7777',
  consumers: ['llm-router'],
}

const context = {
  consoleConfig: null,
  signal: { cancelled: false },
  report: () => undefined,
}

function storeStep(
  input: Extract<PlanStep, { kind: 'store-secret' }>['input'],
): PlanStep {
  return {
    kind: 'store-secret',
    name: 'DEEPSEEK_API_KEY',
    input,
    from: 'this project’s .env.staging',
    consumers: ['llm-router'],
    envFile: '.env.staging',
  }
}

describe('runStep store-secret', () => {
  beforeEach(() => {
    trigger.mockReset()
    trigger.mockImplementation(async (fn: string) =>
      fn === 'secrets::get' ? null : META,
    )
  })

  it('says who may read a variable shared as it is, not that it was stored', async () => {
    const result = await runStep(storeStep({ mode: 'env' }), context)
    expect(result.note).toBe('llm-router can read it')
    expect(trigger).toHaveBeenCalledWith(
      'secrets::access',
      { name: 'DEEPSEEK_API_KEY', consumers: ['llm-router'], store: 'env' },
      expect.anything(),
    )
  })

  it('names the env file a pasted key was written to', async () => {
    const result = await runStep(
      storeStep({ mode: 'paste', value: 'sk-e2e-pasted-7777', store: 'env' }),
      context,
    )
    expect(result.note).toBe('written to .env.staging (sk-e2e…7777)')
  })

  it('keeps saying stored for the encrypted store', async () => {
    const result = await runStep(
      storeStep({ mode: 'paste', value: 'sk-e2e-pasted-7777' }),
      context,
    )
    expect(result.note).toBe('stored sk-e2e…7777')
  })
})
