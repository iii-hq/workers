import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  fetchExamplePrompts,
  type PromptModel,
  parseExamplePrompts,
  resolvePromptModel,
} from './prompts'

const trigger = vi.fn()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger }),
}))

const PRIORITY: PromptModel[] = [
  { provider: 'claude-code', model: 'claude-sonnet-5-5', effort: 'medium' },
  { provider: 'anthropic', model: 'claude-sonnet-5-5', effort: 'medium' },
  { provider: 'openai-codex', model: 'gpt-6.1-sol', effort: 'high' },
  { provider: 'openai', model: 'gpt-6.1-sol' },
]

describe('resolvePromptModel', () => {
  it('takes the first entry this machine has, in the order the prompt lists them', () => {
    expect(
      resolvePromptModel(PRIORITY, [
        'openai::gpt-6.1-sol',
        'anthropic::claude-sonnet-5-5',
        'claude-code::claude-code/claude-sonnet-5-5',
      ]),
    ).toEqual({
      model: 'claude-code::claude-code/claude-sonnet-5-5',
      effort: 'medium',
    })
  })

  it('matches a provider-prefixed id by its last segment, and a bare id exactly', () => {
    expect(
      resolvePromptModel(PRIORITY, [
        'openai-codex::codex/gpt-6.1-sol',
        'anthropic::claude-sonnet-5-5-latest',
      ]),
    ).toEqual({ model: 'openai-codex::codex/gpt-6.1-sol', effort: 'high' })
  })

  it('never matches another provider serving the same model id', () => {
    expect(
      resolvePromptModel(
        [{ provider: 'anthropic', model: 'claude-sonnet-5-5' }],
        ['openrouter::openrouter/anthropic/claude-sonnet-5-5'],
      ),
    ).toBeNull()
  })

  it('leaves the effort out when the entry has none', () => {
    expect(resolvePromptModel(PRIORITY, ['openai::gpt-6.1-sol'])).toEqual({
      model: 'openai::gpt-6.1-sol',
    })
  })

  it('is null when nothing in the list is available, so the chat keeps its default', () => {
    expect(resolvePromptModel(PRIORITY, ['deepseek::deepseek-flash'])).toBe(
      null,
    )
    expect(resolvePromptModel([], ['openai::gpt-6.1-sol'])).toBeNull()
    expect(resolvePromptModel(PRIORITY, ['not-a-key'])).toBeNull()
  })
})

describe('parseExamplePrompts', () => {
  it('keeps the usable entries and drops the rest', () => {
    expect(
      parseExamplePrompts({
        prompts: [
          {
            title: ' Build a TODO app ',
            description: 'A todo list',
            agent: 'ade-worker-builder',
            prompt: 'Build a TODO app.',
            models: [
              { provider: 'claude-code', model: 'claude-sonnet-5-5' },
              { provider: 'openai' },
              'nonsense',
            ],
          },
          { title: 'No prompt', agent: 'default' },
          { prompt: 'No title', agent: 'default' },
          null,
        ],
      }),
    ).toEqual([
      {
        title: 'Build a TODO app',
        description: 'A todo list',
        agent: 'ade-worker-builder',
        prompt: 'Build a TODO app.',
        models: [{ provider: 'claude-code', model: 'claude-sonnet-5-5' }],
      },
    ])
  })

  it('reads anything else as no prompts', () => {
    expect(parseExamplePrompts(null)).toEqual([])
    expect(parseExamplePrompts({ prompts: 'x' })).toEqual([])
  })
})

describe('fetchExamplePrompts', () => {
  beforeEach(() => {
    trigger.mockReset()
  })

  it('asks the ADE backend for the project’s prompts', async () => {
    trigger.mockResolvedValue({
      prompts: [{ title: 'T', agent: 'default', prompt: 'P', models: [] }],
    })
    await expect(fetchExamplePrompts()).resolves.toEqual([
      { title: 'T', agent: 'default', prompt: 'P', models: [] },
    ])
    expect(trigger).toHaveBeenCalledWith(
      'console::onboarding::prompts',
      {},
      expect.anything(),
    )
  })

  it('shows none on a backend that predates them', async () => {
    trigger.mockImplementation(async () => {
      throw { code: 'function_not_found' }
    })
    await expect(fetchExamplePrompts()).resolves.toEqual([])
  })
})
