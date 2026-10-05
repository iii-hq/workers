import { describe, expect, it } from 'vitest'
import {
  type CatalogModelRow,
  catalogRowsToModelOptions,
  type ProviderListEntry,
  preferredStartingModel,
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

describe('preferredStartingModel', () => {
  const provider = (
    id: string,
    extra: Partial<ProviderListEntry> = {},
  ): ProviderListEntry => ({
    id,
    display_name: id,
    supports_model_listing: true,
    configured: true,
    available: true,
    ...extra,
  })

  it('takes the first configured, available provider with a listed default', () => {
    const providers = [
      provider('openai', { default_model: 'gpt-6.1-sol' }),
      provider('anthropic', { default_model: 'claude-sonnet-5-5' }),
    ]
    const keys = new Set([
      'anthropic::claude-sonnet-5-5',
      'openai::gpt-6.1-sol',
    ])
    expect(preferredStartingModel(providers, keys)).toBe(
      'anthropic::claude-sonnet-5-5',
    )
  })

  it('skips providers that are unconfigured, unavailable, defaultless or unlisted', () => {
    const providers = [
      provider('anthropic', {
        default_model: 'claude-sonnet-5-5',
        configured: false,
      }),
      provider('deepseek', {
        default_model: 'deepseek-flash',
        available: false,
      }),
      provider('kimi'),
      provider('openai', { default_model: 'gpt-6.1-sol' }),
      provider('xai', { default_model: 'grok-4.3' }),
    ]
    const keys = new Set([
      'anthropic::claude-sonnet-5-5',
      'deepseek::deepseek-flash',
      'kimi::kimi-k3',
      'xai::grok-4.3',
    ])
    expect(preferredStartingModel(providers, keys)).toBe('xai::grok-4.3')
    expect(preferredStartingModel(providers, new Set())).toBeNull()
  })
})
