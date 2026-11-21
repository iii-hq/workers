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

describe('createApi', () => {
  it('reads the entry raw under the id the worker reports', async () => {
    const { iii, calls } = fakeIii({
      'mcp-client::configuration-id': () => ({ id: 'mcp-client-dev' }),
      'configuration::get': get,
    })
    expect(await createApi(iii).readConfig()).toEqual({ id: 'mcp-client-dev', mcpServers: stored.mcpServers })
    expect(calls[1]).toMatchObject({ fn: 'configuration::get', payload: { id: 'mcp-client-dev', raw: true } })
  })

  it('falls back to the id mcp-client when the worker is not running', async () => {
    const { iii, calls } = fakeIii({ 'configuration::get': get })
    const api = createApi(iii)
    expect((await api.readConfig()).id).toBe('mcp-client')
    expect(calls.at(-1)?.payload).toEqual({ id: 'mcp-client', raw: true })
    // The fallback is not kept: the next read asks the worker again.
    await api.readConfig()
    expect(calls.filter((c) => c.fn === 'mcp-client::configuration-id')).toHaveLength(2)
  })

  it('reads a missing entry and a root that is not an object as no servers', async () => {
    const missing = fakeIii({
      'configuration::get': () => {
        throw { code: 'NOT_FOUND', message: 'no entry mcp-client' }
      },
    })
    expect((await createApi(missing.iii).readConfig()).mcpServers).toEqual({})
    // biome-ignore lint/suspicious/noTemplateCurlyInString: an unresolved reference, stored as text
    const text = fakeIii({ 'configuration::get': () => ({ value: '${X}' }) })
    expect((await createApi(text.iii).readConfig()).mcpServers).toEqual({})
  })

  it('writeEntry re-reads and changes only that entry', async () => {
    const { iii, calls } = fakeIii({ 'configuration::get': get, 'configuration::set': () => ({}) })
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

  it('call uses the real function id and a 130 s timeout', async () => {
    const { iii, calls } = fakeIii({ 'mcp::linear::whoami': () => ({ content: [] }) })
    await createApi(iii).call('mcp::linear::whoami', {})
    expect(calls[0]).toEqual({ fn: 'mcp::linear::whoami', payload: {}, options: { timeoutMs: 130000 } })
  })
})
