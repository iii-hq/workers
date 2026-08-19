import { describe, expect, it, vi } from 'vitest'
import {
  getCommandPrompt,
  getPrompt,
  getSkill,
  listCommandPrompts,
  listPrompts,
  listSkills,
} from './directory-prompts'

const TIMEOUT = { timeoutMs: 10_000 }

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
      TIMEOUT,
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
      { name: 'pirate' },
      TIMEOUT,
    )
  })

  it('listCommandPrompts targets the command-prompt family', async () => {
    const entries = [{ name: 'review', description: 'R.', modified_at: 't' }]
    const client = fakeClient(() => ({ prompts: entries }))
    expect(await listCommandPrompts(client)).toEqual(entries)
    expect(client.trigger).toHaveBeenCalledWith(
      'directory::prompts::list',
      {},
      TIMEOUT,
    )
  })

  it('getCommandPrompt passes the name through', async () => {
    const client = fakeClient(() => ({
      name: 'review',
      description: 'R.',
      body: 'Check.',
      modified_at: 't',
    }))
    expect((await getCommandPrompt(client, 'review')).body).toBe('Check.')
    expect(client.trigger).toHaveBeenCalledWith(
      'directory::prompts::get',
      { name: 'review' },
      TIMEOUT,
    )
  })

  it('listSkills requests descriptions and unwraps the skills array', async () => {
    const skills = [
      {
        id: 'coder/index',
        title: 'coder',
        description: 'D.',
        modified_at: 't',
      },
    ]
    const client = fakeClient(() => ({ skills }))
    expect(await listSkills(client)).toEqual(skills)
    expect(client.trigger).toHaveBeenCalledWith(
      'directory::skills::list',
      { include_description: true },
      TIMEOUT,
    )
  })

  it('getSkill passes the id through', async () => {
    const client = fakeClient(() => ({
      id: 'coder/index',
      title: 'coder',
      body: 'Skill.',
      modified_at: 't',
    }))
    expect((await getSkill(client, 'coder/index')).body).toBe('Skill.')
    expect(client.trigger).toHaveBeenCalledWith(
      'directory::skills::get',
      { id: 'coder/index' },
      TIMEOUT,
    )
  })
})
