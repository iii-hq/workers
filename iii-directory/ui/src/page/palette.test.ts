import type { Host } from '@iii-dev/console-ui'
import { expect, it, vi } from 'vitest'
import { registerDirectoryPalette } from './palette'

it('searches skills, system prompts, and agents without command prompts', async () => {
  const trigger = vi.fn(async (functionId: string) => {
    if (functionId === 'directory::skills::list') {
      return { skills: [{ id: 'one', title: 'One', description: '' }] }
    }
    if (functionId === 'directory::system-prompts::list') {
      return { prompts: [{ name: 'two', description: '' }] }
    }
    if (functionId === 'directory::agents::list') {
      return { agents: [{ id: 'three', name: 'Three', description: '' }] }
    }
    throw new Error(`unexpected function: ${functionId}`)
  })
  const registerSource = vi.fn()
  const host = {
    iii: { trigger },
    palette: { registerSource },
    commands: { register: vi.fn() },
  } as unknown as Host

  registerDirectoryPalette(host)
  const source = registerSource.mock.calls[0][0] as {
    search: (query: string, options: { signal: AbortSignal }) => Promise<unknown>
  }

  await expect(
    source.search('one', { signal: new AbortController().signal }),
  ).resolves.toHaveLength(1)
  expect(trigger).not.toHaveBeenCalledWith('directory::prompts::list', {})
})
