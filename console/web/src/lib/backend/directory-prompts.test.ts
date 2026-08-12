import { describe, expect, it, vi } from 'vitest'
import { getPrompt, listPrompts } from './directory-prompts'

function fakeClient(impl?: (fn: string, payload: unknown) => unknown) {
  const trigger = vi.fn(async (fn: string, payload: unknown) =>
    impl ? impl(fn, payload) : undefined,
  )
  return {
    trigger: trigger as unknown as <T>(fn: string, p?: object) => Promise<T>,
    // biome-ignore lint/suspicious/noExplicitAny: test double
  } as any
}

describe('client calls', () => {
  it('listPrompts unwraps the prompts array', async () => {
    const entries = [{ name: 'pirate', description: 'P.', modified_at: 't' }]
    const client = fakeClient(() => ({ prompts: entries }))
    expect(await listPrompts(client)).toEqual(entries)
    expect(client.trigger).toHaveBeenCalledWith(
      'directory::system-prompts::list',
      {},
    )
  })

  it('getPrompt passes the name through', async () => {
    const client = fakeClient(() => ({
      name: 'pirate',
      description: 'P.',
      body: 'Arr.',
      modified_at: 't',
    }))
    expect((await getPrompt(client, 'pirate')).body).toBe('Arr.')
    expect(client.trigger).toHaveBeenCalledWith(
      'directory::system-prompts::get',
      {
        name: 'pirate',
      },
    )
  })
})
