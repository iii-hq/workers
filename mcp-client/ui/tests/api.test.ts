import type { ExtensionIii } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { createApi } from '../src/api'

type Payload = Record<string, unknown>
type Answers = Record<string, (payload: Payload) => unknown>

/** An `iii` that answers from `answers` and records every call; an id with
 * no answer rejects the way the engine does for an unregistered function. */
function fakeIii(answers: Answers) {
  const calls: { fn: string; payload: Payload; options?: { timeoutMs?: number } }[] = []
  const call = async (fn: string, payload: Payload = {}, options?: { timeoutMs?: number }) => {
    calls.push({ fn, payload, options })
    const answer = answers[fn]
    if (!answer) throw { code: 'function_not_found', message: `function ${fn} not found` }
    return answer(payload)
  }
  return { iii: { call } as unknown as ExtensionIii, calls }
}

const stored = {
  mcpServers: { linear: { url: 'https://mcp.linear.app/mcp' }, docs: { command: 'npx' } },
  other: 1,
}
const get = () => ({ value: structuredClone(stored) })
const ID = { 'mcp-client::configuration-id': () => ({ id: 'mcp-client' }) }

describe('createApi', () => {
  it('reads the entry raw under the id the worker reports', async () => {
    const { iii, calls } = fakeIii({
      'mcp-client::configuration-id': () => ({ id: 'mcp-client-dev' }),
      'configuration::get': get,
    })
    expect(await createApi(iii).readConfig()).toEqual({ id: 'mcp-client-dev', mcpServers: stored.mcpServers })
    expect(calls[1]).toMatchObject({ fn: 'configuration::get', payload: { id: 'mcp-client-dev', raw: true } })
  })

  it('never guesses the id: without one it reads and writes nothing, and asks again next time', async () => {
    const { iii, calls } = fakeIii({ 'configuration::get': get, 'configuration::set': () => ({}) })
    const api = createApi(iii)
    await expect(api.readConfig()).rejects.toThrow()
    await expect(api.writeEntry('linear', null)).rejects.toThrow()
    expect(calls.map((c) => c.fn)).toEqual(['mcp-client::configuration-id', 'mcp-client::configuration-id'])
  })

  it('reads a missing entry and a root that is not an object as no servers', async () => {
    const missing = fakeIii({
      ...ID,
      'configuration::get': () => {
        throw { code: 'NOT_FOUND', message: 'no entry mcp-client' }
      },
    })
    expect((await createApi(missing.iii).readConfig()).mcpServers).toEqual({})
    // biome-ignore lint/suspicious/noTemplateCurlyInString: an unresolved reference, stored as text
    const text = fakeIii({ ...ID, 'configuration::get': () => ({ value: '${X}' }) })
    expect((await createApi(text.iii).readConfig()).mcpServers).toEqual({})
  })

  it('writeEntry re-reads and changes only that entry', async () => {
    const { iii, calls } = fakeIii({ ...ID, 'configuration::get': get, 'configuration::set': () => ({}) })
    const api = createApi(iii)
    await api.writeEntry('docs', { command: 'uvx' })
    expect(calls.at(-1)).toEqual({
      fn: 'configuration::set',
      payload: {
        id: 'mcp-client',
        value: { mcpServers: { linear: stored.mcpServers.linear, docs: { command: 'uvx' } }, other: 1 },
      },
    })
    await api.writeEntry('linear', null)
    expect(calls.at(-1)?.payload.value).toEqual({ mcpServers: { docs: stored.mcpServers.docs }, other: 1 })
  })

  it('listFunctions keeps mcp:: ids, and one server by exact segment', async () => {
    const ids = ['mcp::linear::whoami', 'mcp::linear-app::whoami', 'mcp-client::servers::list', 'state::get']
    const { iii } = fakeIii({
      'engine::functions::list': () => ({ functions: ids.map((function_id) => ({ function_id })) }),
    })
    const api = createApi(iii)
    expect((await api.listFunctions()).map((f) => f.function_id)).toEqual([
      'mcp::linear::whoami',
      'mcp::linear-app::whoami',
    ])
    expect((await api.listFunctions('linear')).map((f) => f.function_id)).toEqual(['mcp::linear::whoami'])
  })

  it('shareSecret adds mcp-client and keeps every consumer that was there', async () => {
    const { iii, calls } = fakeIii({
      'secrets::get': () => ({ consumers: ['database'] }),
      'secrets::access': () => ({}),
    })
    await createApi(iii).shareSecret('secret://DB_KEY')
    const sent = calls.map((c) => [c.fn, c.payload])
    expect(sent).toEqual([
      ['secrets::get', { name: 'DB_KEY' }],
      ['secrets::access', { name: 'DB_KEY', consumers: ['database', 'mcp-client'] }],
    ])
  })

  it('shareSecret names the env store for an env reference', async () => {
    const { iii, calls } = fakeIii({
      'secrets::get': () => ({ consumers: [] }),
      'secrets::access': () => ({}),
    })
    await createApi(iii).shareSecret('env://GH_TOKEN')
    const sent = calls.map((c) => [c.fn, c.payload])
    expect(sent).toEqual([
      ['secrets::get', { name: 'GH_TOKEN', store: 'env' }],
      ['secrets::access', { name: 'GH_TOKEN', consumers: ['mcp-client'], store: 'env' }],
    ])
  })

  it('shareSecret writes nothing when the read fails', async () => {
    const { iii, calls } = fakeIii({
      'secrets::get': () => {
        throw new Error('no such secret')
      },
    })
    await expect(createApi(iii).shareSecret('secret://GONE')).rejects.toThrow('no such secret')
    expect(calls.map((c) => c.fn)).toEqual(['secrets::get'])
  })

  it('shareSecret reads no grant on an env variable as none, and shares it', async () => {
    const { iii, calls } = fakeIii({ 'secrets::get': () => null, 'secrets::access': () => ({}) })
    await createApi(iii).shareSecret('env://GH_TOKEN')
    expect(calls.map((c) => [c.fn, c.payload])).toEqual([
      ['secrets::get', { name: 'GH_TOKEN', store: 'env' }],
      ['secrets::access', { name: 'GH_TOKEN', consumers: ['mcp-client'], store: 'env' }],
    ])
  })

  it('shareSecret refuses a vault name with no secret and writes nothing', async () => {
    const { iii, calls } = fakeIii({ 'secrets::get': () => null, 'secrets::access': () => ({}) })
    await expect(createApi(iii).shareSecret('secret://GONE')).rejects.toThrow('`GONE` is not in the vault.')
    expect(calls.map((c) => c.fn)).toEqual(['secrets::get'])
  })

  it('storeSecret never replaces a secret another worker reads, and stores nothing', async () => {
    const { iii, calls } = fakeIii({
      'secrets::get': () => ({ consumers: ['llm-router', 'mcp-client'] }),
      'secrets::set': () => ({}),
    })
    await expect(createApi(iii).storeSecret('A', 'v')).rejects.toThrow(
      'Vault name `A` already holds a secret that llm-router reads. Nothing was stored.',
    )
    expect(calls.map((c) => c.fn)).toEqual(['secrets::get'])
  })

  it('storeSecret replaces a secret only mcp-client reads, keeping its description', async () => {
    const { iii, calls } = fakeIii({
      'secrets::get': () => ({ consumers: ['mcp-client'] }),
      'secrets::set': () => ({}),
    })
    await createApi(iii).storeSecret('A', 'v')
    expect(calls.map((c) => [c.fn, c.payload])).toEqual([
      ['secrets::get', { name: 'A' }],
      ['secrets::set', { name: 'A', value: 'v', consumers: ['mcp-client'] }],
    ])
  })

  it('storeSecret stores a new name with its description', async () => {
    const { iii, calls } = fakeIii({ 'secrets::get': () => null, 'secrets::set': () => ({}) })
    await createApi(iii).storeSecret('A', 'v')
    expect(calls.map((c) => [c.fn, c.payload])).toEqual([
      ['secrets::get', { name: 'A' }],
      [
        'secrets::set',
        { name: 'A', value: 'v', consumers: ['mcp-client'], description: 'Stored by the MCP client page' },
      ],
    ])
  })

  it('secretReaders reads who reads a vault name: null when it holds nothing', async () => {
    const api = (answer: unknown) => createApi(fakeIii({ 'secrets::get': () => answer }).iii)
    expect(await api(null).secretReaders('A')).toBeNull()
    expect(await api({ consumers: ['llm-router'] }).secretReaders('A')).toEqual(['llm-router'])
    expect(await api({}).secretReaders('A')).toEqual([])
    await expect(api({ consumers: 'x' }).secretReaders('A')).rejects.toThrow('returned no consumer list for A')
    await expect(createApi(fakeIii({}).iii).secretReaders('A')).rejects.toThrow('The secrets worker is not running')
  })

  it('storeSecret says the secrets worker is not running', async () => {
    const { iii, calls } = fakeIii({})
    await expect(createApi(iii).storeSecret('A', 'v')).rejects.toThrow(
      'The secrets worker is not running. Start it, then import again.',
    )
    expect(calls.map((c) => c.fn)).toEqual(['secrets::get'])
  })

  it('shareEnv shares the env variable through the env store', async () => {
    const { iii, calls } = fakeIii({
      'secrets::get': () => ({ consumers: ['database'] }),
      'secrets::access': () => ({}),
    })
    await createApi(iii).shareEnv('GH_TOKEN')
    expect(calls.map((c) => [c.fn, c.payload])).toEqual([
      ['secrets::get', { name: 'GH_TOKEN', store: 'env' }],
      ['secrets::access', { name: 'GH_TOKEN', consumers: ['database', 'mcp-client'], store: 'env' }],
    ])
  })

  it('call uses the real function id and a 130 s timeout', async () => {
    const { iii, calls } = fakeIii({ 'mcp::linear::whoami': () => ({ content: [] }) })
    await createApi(iii).call('mcp::linear::whoami', {})
    expect(calls[0]).toEqual({ fn: 'mcp::linear::whoami', payload: {}, options: { timeoutMs: 130000 } })
  })
})
