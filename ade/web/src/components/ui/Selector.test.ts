import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { filterSelectorGroups, Selector } from './Selector'

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

  it('forwards reusable field identity and validation to its trigger', () => {
    const html = renderToStaticMarkup(
      createElement(Selector, {
        id: 'provider',
        name: 'provider.id',
        'data-field': 'provider.id',
        'aria-label': 'Provider',
        'aria-invalid': true,
        'aria-describedby': 'provider-error',
        value: 'openai',
        options: [{ value: 'openai', label: 'OpenAI' }],
        onChange: () => {},
      }),
    )

    expect(html).toContain('type="hidden"')
    expect(html).toContain('name="provider.id"')
    expect(html).toContain('id="provider"')
    expect(html).toContain('data-field="provider.id"')
    expect(html).toContain('aria-invalid="true"')
    expect(html).toContain('aria-describedby="provider-error"')
  })
})
