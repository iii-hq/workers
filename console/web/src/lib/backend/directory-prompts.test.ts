import { describe, expect, it, vi } from 'vitest'
import {
  getCommandPrompt,
  getPrompt,
  getSkill,
  listCommandPrompts,
  listPrompts,
  listSkills,
  skillBodyWithBaseDir,
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

describe('skillBodyWithBaseDir', () => {
  const skill = {
    id: 'impeccable',
    title: 'Impeccable',
    body: 'Run scripts/context.mjs.',
    modified_at: 't',
  }

  it('appends the base directory when the worker sends a path', () => {
    const out = skillBodyWithBaseDir({
      ...skill,
      path: '/home/u/.agents/skills/impeccable/SKILL.md',
    })
    expect(out).toContain('Run scripts/context.mjs.')
    expect(out).toContain(
      'Skill base directory: /home/u/.agents/skills/impeccable',
    )
  })

  it('is body-only when the worker predates the path field', () => {
    expect(skillBodyWithBaseDir(skill)).toBe('Run scripts/context.mjs.')
  })

  it('handles a Windows path from a Windows worker build', () => {
    const out = skillBodyWithBaseDir({
      ...skill,
      path: 'C:\\Users\\u\\.agents\\skills\\impeccable\\SKILL.md',
    })
    expect(out).toContain(
      'Skill base directory: C:\\Users\\u\\.agents\\skills\\impeccable',
    )
  })

  it('falls back to body-only rather than emit a truncated path', () => {
    // No separator at all: naive `slice(0, lastIndexOf(...))` would shave the
    // final character off and present the result as a directory.
    expect(skillBodyWithBaseDir({ ...skill, path: 'SKILL.md' })).toBe(
      'Run scripts/context.mjs.',
    )
    expect(skillBodyWithBaseDir({ ...skill, path: '/SKILL.md' })).toBe(
      'Run scripts/context.mjs.',
    )
  })
})
