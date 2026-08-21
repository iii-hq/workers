import type { FunctionTriggerMessage } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { createSearchTriggerRenderer } from './search-card'

vi.mock('@iii-dev/console-ui', () => ({
  Badge: () => null,
}))

const output = {
  guidance: 'Choose the smallest candidate set.',
  workers: [
    {
      namespace: 'browser',
      functions: [
        {
          function_id: 'browser::fetch',
          description: 'Fetch a web page.',
        },
      ],
    },
  ],
  latency_ms: 12,
}

describe('search trigger renderer', () => {
  it('keeps the custom result hidden until the host card expands', () => {
    expect(createSearchTriggerRenderer().metadata?.display).not.toBe(true)
  })

  it('renders submitted capabilities in the expanded result', () => {
    const rendered = createSearchTriggerRenderer().tryRender({
      functionId: 'directory::search_functions',
      input: {
        query: 'Fetch and summarize the latest news.',
        capabilities: [
          'fetch webpage content from g1.globo.com',
          'extract latest news headlines',
        ],
      },
      output,
    } as FunctionTriggerMessage) as {
      type: (props: Record<string, unknown>) => unknown
      props: Record<string, unknown>
    }

    const card = rendered.type(rendered.props)
    const serialized = JSON.stringify(card)
    expect(serialized).toContain('fetch webpage content from g1.globo.com')
    expect(serialized).toContain('extract latest news headlines')
  })
})
