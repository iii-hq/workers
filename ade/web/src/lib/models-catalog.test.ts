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

  /* The send path refuses to hand a picture to a model that cannot see one, so
     this flag has to survive the catalog. It stays TRI-state: a router that
     says nothing must not read as "no", or every model on an older catalog
     would start rejecting images. */
  it('carries vision support through, including "not stated"', () => {
    const rows: CatalogModelRow[] = [
      {
        id: 'deepseek-v4-flash',
        provider: 'deepseek',
        display_name: 'DeepSeek V4 Flash',
        supports_vision: false,
      },
      {
        id: 'claude-haiku-4-5',
        provider: 'anthropic',
        display_name: 'Claude Haiku 4.5',
        supports_vision: true,
      },
      {
        id: 'mystery-1',
        provider: 'somewhere',
        display_name: 'Mystery 1',
      },
    ]

    const byId = new Map(
      catalogRowsToModelOptions(rows).map((o) => [o.id, o.supportsVision]),
    )
    expect(byId.get('deepseek::deepseek-v4-flash')).toBe(false)
    expect(byId.get('anthropic::claude-haiku-4-5')).toBe(true)
    expect(byId.get('somewhere::mystery-1')).toBeUndefined()
  })
})
