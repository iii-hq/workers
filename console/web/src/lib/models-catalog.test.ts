import { describe, expect, it } from 'vitest'
import {
  type CatalogModelRow,
  catalogRowsToModelOptions,
  parseProviderList,
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

describe('parseProviderList', () => {
  it('uses explicit provider status and exposes quarantined worker conflicts', () => {
    expect(
      parseProviderList(
        [
          {
            id: 'kimi',
            display_name: 'Kimi',
            worker_name: 'provider-kimi',
            status: 'needs_configuration',
            configured: false,
            available: true,
          },
        ],
        [{ name: 'provider-kimi', quarantined: true }],
      ),
    ).toEqual([
      expect.objectContaining({
        id: 'kimi',
        status: 'needs_configuration',
        configured: false,
        available: true,
        conflicted: true,
      }),
    ])
  })

  it('derives status for older routers without the explicit field', () => {
    expect(
      parseProviderList(
        [
          { id: 'down', available: false },
          { id: 'keyless', configured: false },
          { id: 'ready' },
        ],
        [],
      ).map(({ id, status }) => ({ id, status })),
    ).toEqual([
      { id: 'down', status: 'unavailable' },
      { id: 'keyless', status: 'needs_configuration' },
      { id: 'ready', status: 'ready' },
    ])
  })
})
