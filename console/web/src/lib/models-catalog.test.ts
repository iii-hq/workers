import { describe, expect, it } from 'vitest'
import {
  type CatalogModelRow,
  catalogRowsToModelOptions,
} from './models-catalog'

describe('catalogRowsToModelOptions', () => {
  it('preserves model-specific effort order and descriptions', () => {
    const rows: CatalogModelRow[] = [
      {
        id: 'codex/gpt-5.6-sol',
        provider: 'openai-codex',
        display_name: 'GPT-5.6-Sol (Codex)',
        supports_thinking: true,
        reasoning_efforts: [
          { effort: 'low', description: 'fast responses' },
          { effort: 'xhigh', description: 'extra high reasoning' },
          { effort: 'ultra', description: 'delegated reasoning' },
        ],
      },
    ]

    expect(catalogRowsToModelOptions(rows)[0]).toMatchObject({
      id: 'openai-codex::codex/gpt-5.6-sol',
      reasoningEfforts: [
        { effort: 'low', description: 'fast responses' },
        { effort: 'xhigh', description: 'extra high reasoning' },
        { effort: 'ultra', description: 'delegated reasoning' },
      ],
    })
  })
})
