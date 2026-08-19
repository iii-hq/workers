import { describe, expect, it } from 'vitest'
import { filterSelectorGroups } from './Selector'

const groups = [
  {
    label: 'providers',
    options: [
      {
        value: 'anthropic',
        label: 'Claude Sonnet',
        description: 'Anthropic model',
        keywords: ['reasoning'],
      },
      { value: 'openai', label: 'GPT', description: 'OpenAI model' },
    ],
  },
  {
    label: 'local',
    options: [{ value: 'ollama', label: 'Ollama', disabled: true }],
  },
] as const

describe('filterSelectorGroups', () => {
  it('matches labels, descriptions and keywords case-insensitively', () => {
    expect(
      filterSelectorGroups({ groups, query: 'REASONING' }).flatMap((group) =>
        group.options.map(({ option }) => option.value),
      ),
    ).toEqual(['anthropic'])
    expect(
      filterSelectorGroups({ groups, query: 'openai model' }).flatMap((group) =>
        group.options.map(({ option }) => option.value),
      ),
    ).toEqual(['openai'])
  })

  it('removes empty groups while preserving option state', () => {
    const result = filterSelectorGroups({ groups, query: 'ollama' })
    expect(result).toHaveLength(1)
    expect(result[0].label).toBe('local')
    expect(result[0].options[0].option.disabled).toBe(true)
  })

  it('can delegate filtering to an async caller', () => {
    const result = filterSelectorGroups({
      groups,
      query: 'no local match',
      shouldFilter: false,
    })
    expect(result.flatMap((group) => group.options)).toHaveLength(3)
  })
})
