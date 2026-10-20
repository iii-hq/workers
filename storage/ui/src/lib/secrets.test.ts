import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import {
  envRefName,
  envStatusText,
  getSecret,
  mergeConsumers,
  parseSecretMeta,
  secretRef,
  secretRefName,
  secretStatusText,
  storeSecret,
  updatedLabel,
} from './secrets'

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
    expect(secretRef('STORAGE_PHOTOS_SECRET_ACCESS_KEY')).toBe('secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY')
    expect(secretRefName(' secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY ')).toBe('STORAGE_PHOTOS_SECRET_ACCESS_KEY')
    expect(secretRefName('Secret://X')).toBe('X')
    expect(secretRefName('secret://9BAD')).toBeNull()
    expect(secretRefName('secret://')).toBeNull()
    expect(secretRefName('hunter2')).toBeNull()
    expect(secretRefName(7)).toBeNull()
    expect(envRefName('env://AWS_SECRET_ACCESS_KEY')).toBe('AWS_SECRET_ACCESS_KEY')
    expect(envRefName('secret://AWS_SECRET_ACCESS_KEY')).toBeNull()
  })

  it('keeps the consumers a secret already has', () => {
    expect(mergeConsumers(['database'], ['storage'])).toEqual(['database', 'storage'])
    expect(mergeConsumers(['storage'], ['storage'])).toEqual(['storage'])
    expect(mergeConsumers(undefined, ['storage'])).toEqual(['storage'])
  })

  it('parses metadata leniently', () => {
    expect(parseSecretMeta(null)).toBeNull()
    expect(parseSecretMeta({ hint: 'x' })).toBeNull()
    expect(parseSecretMeta({ name: 'N', consumers: ['storage', 3], updated_at: '2026-10-20T10:00:00Z' })).toEqual({
      name: 'N',
      consumers: ['storage'],
      store: undefined,
      hint: undefined,
      description: undefined,
      updated_at: '2026-10-20T10:00:00Z',
    })
  })
})

describe('the secrets worker', () => {
  it('stores a value for the storage worker, merging consumers', async () => {
    const { host, calls } = fakeHost((fn) =>
      fn === 'secrets::get'
        ? { name: 'STORAGE_PHOTOS_SECRET_ACCESS_KEY', hint: 'wJa…EKEY', consumers: ['backup'] }
        : { name: 'STORAGE_PHOTOS_SECRET_ACCESS_KEY' },
    )
    const ref = await storeSecret(host, {
      name: 'STORAGE_PHOTOS_SECRET_ACCESS_KEY',
      value: 'k',
      description: 'Secret access key',
    })
    expect(ref).toBe('secret://STORAGE_PHOTOS_SECRET_ACCESS_KEY')
    expect(calls.map((c) => c.fn)).toEqual(['secrets::get', 'secrets::set'])
    expect(calls[0].payload).toEqual({ name: 'STORAGE_PHOTOS_SECRET_ACCESS_KEY' })
    expect(calls[1].payload).toEqual({
      name: 'STORAGE_PHOTOS_SECRET_ACCESS_KEY',
      value: 'k',
      consumers: ['backup', 'storage'],
      description: 'Secret access key',
    })
  })

  it('creates a new secret readable by the storage worker', async () => {
    const { host, calls } = fakeHost((fn) => (fn === 'secrets::get' ? null : {}))
    await storeSecret(host, { name: 'N', value: 'v', description: 'd' })
    expect(calls[1].payload.consumers).toEqual(['storage'])
  })

  it('asks the env store for an env:// variable', async () => {
    const { host, calls } = fakeHost(() => ({ name: 'AWS_SECRET_ACCESS_KEY', store: 'env', hint: '' }))
    const meta = await getSecret(host, 'AWS_SECRET_ACCESS_KEY', 'env')
    expect(calls[0].payload).toEqual({ name: 'AWS_SECRET_ACCESS_KEY', store: 'env' })
    expect(meta?.store).toBe('env')
  })

  it('tells a missing secret from a missing worker', async () => {
    const absent = fakeHost(() => null)
    expect(await getSecret(absent.host, 'N')).toBeNull()

    const down = fakeHost(() => {
      throw { code: 'function_not_found', message: 'Function secrets::get not found' }
    })
    expect(await getSecret(down.host, 'N')).toBeUndefined()
    await expect(storeSecret(down.host, { name: 'N', value: 'v', description: 'd' })).rejects.toThrow(
      /secrets worker is not running/,
    )
  })

  it('passes other failures through', async () => {
    const denied = fakeHost(() => {
      throw { code: 'FORBIDDEN', message: 'denied' }
    })
    await expect(getSecret(denied.host, 'N')).rejects.toEqual({ code: 'FORBIDDEN', message: 'denied' })
  })
})

describe('status lines', () => {
  const now = Date.parse('2026-10-20T12:00:00Z')

  it('never shows the hint of a stored secret', () => {
    const meta = { name: 'K', consumers: ['storage'], hint: 'wJa…EKEY', updated_at: '2026-10-20T11:55:00Z' }
    const line = secretStatusText(meta, true, now)
    expect(line).toBe('Stored in secrets · updated 5m ago')
    expect(line).not.toContain('wJa')
    expect(secretStatusText({ name: 'K', consumers: [] }, true, now)).toBe('Stored in secrets')
    expect(secretStatusText(null, true, now)).toBe('Missing from the secrets store — the bucket cannot resolve it')
    expect(secretStatusText(undefined, undefined, now)).toBe('Checking the secrets store…')
    expect(secretStatusText(undefined, false, now)).toBe('The secrets worker is not running')
  })

  it('says whether the env file sets a variable, from the hint’s presence alone', () => {
    expect(envStatusText({ name: 'V', consumers: [], hint: 'ab…cd' }, true)).toBe('Set in the secrets worker’s env file')
    expect(envStatusText({ name: 'V', consumers: [], hint: '' }, true)).toBe('Not set in the secrets worker’s env file')
    expect(envStatusText(null, true)).toBe('Not shared with the storage worker')
    expect(envStatusText(undefined, undefined)).toBe('A variable of the secrets worker’s env file')
    expect(envStatusText(undefined, false)).toBe('The secrets worker is not running')
  })

  it('words the age of a secret', () => {
    expect(updatedLabel(undefined, now)).toBeNull()
    expect(updatedLabel('nope', now)).toBeNull()
    expect(updatedLabel('2026-10-20T11:59:40Z', now)).toBe('updated just now')
    expect(updatedLabel('2026-10-20T09:00:00Z', now)).toBe('updated 3h ago')
    expect(updatedLabel('2026-10-15T12:00:00Z', now)).toBe('updated 5d ago')
  })
})
