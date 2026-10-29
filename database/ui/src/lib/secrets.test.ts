import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { envRefName, getSecret, mergeConsumers, secretRef, secretRefName, storeSecret } from './secrets'

function fakeHost(respond: (fn: string, payload: Record<string, unknown>) => unknown) {
  const calls: Array<{ fn: string; payload: Record<string, unknown> }> = []
  const host = {
    iii: {
      trigger: async (fn: string, payload: Record<string, unknown>) => {
        calls.push({ fn, payload })
        return respond(fn, payload)
      },
    },
  } as unknown as Host
  return { host, calls }
}

describe('references', () => {
  it('reads secret:// and env:// names and nothing else', () => {
    expect(secretRef('DATABASE_SHOP_PASSWORD')).toBe('secret://DATABASE_SHOP_PASSWORD')
    expect(secretRefName(' secret://DATABASE_SHOP_PASSWORD ')).toBe('DATABASE_SHOP_PASSWORD')
    expect(secretRefName('Secret://X')).toBe('X')
    expect(secretRefName('secret://9BAD')).toBeNull()
    expect(secretRefName('hunter2')).toBeNull()
    expect(envRefName('env://PGPASSWORD')).toBe('PGPASSWORD')
    expect(envRefName('secret://PGPASSWORD')).toBeNull()
  })

  it('keeps the consumers a secret already has', () => {
    expect(mergeConsumers(['llm-router'], ['database'])).toEqual(['llm-router', 'database'])
    expect(mergeConsumers(['database'], ['database'])).toEqual(['database'])
    expect(mergeConsumers(undefined, ['database'])).toEqual(['database'])
  })
})

describe('the secrets worker', () => {
  it('stores a value for the database worker, merging consumers', async () => {
    const { host, calls } = fakeHost((fn) =>
      fn === 'secrets::get'
        ? { name: 'DATABASE_SHOP_PASSWORD', hint: '••••', consumers: ['backup'] }
        : { name: 'DATABASE_SHOP_PASSWORD' },
    )
    const ref = await storeSecret(host, { name: 'DATABASE_SHOP_PASSWORD', value: 'pw', description: 'Password' })
    expect(ref).toBe('secret://DATABASE_SHOP_PASSWORD')
    expect(calls.map((c) => c.fn)).toEqual(['secrets::get', 'secrets::set'])
    expect(calls[1].payload).toEqual({
      name: 'DATABASE_SHOP_PASSWORD',
      value: 'pw',
      consumers: ['backup', 'database'],
      description: 'Password',
    })
  })

  it('creates a new secret readable by the database worker', async () => {
    const { host, calls } = fakeHost((fn) => (fn === 'secrets::get' ? null : {}))
    await storeSecret(host, { name: 'N', value: 'v', description: 'd' })
    expect(calls[1].payload.consumers).toEqual(['database'])
  })

  it('tells a missing secret from a missing worker', async () => {
    const absent = fakeHost(() => null)
    expect(await getSecret(absent.host, 'N')).toBeNull()

    const down = fakeHost(() => {
      throw { code: 'function_not_found', message: 'function secrets::get not found' }
    })
    expect(await getSecret(down.host, 'N')).toBeUndefined()
    await expect(storeSecret(down.host, { name: 'N', value: 'v', description: 'd' })).rejects.toThrow(
      /secrets worker is not running/,
    )
  })
})
