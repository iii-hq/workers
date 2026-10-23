import { describe, expect, it } from 'vitest'
import {
  type CatalogModelRow,
  catalogKeysInRouterOrder,
  catalogRowsToModelOptions,
  type ProviderListEntry,
  parseProviderList,
  preferredStartingModel,
  rememberedProvider,
  rememberProviderList,
} from './models-catalog'

describe('parseProviderList', () => {
  it('keeps a provider-declared context overflow hint and drops a blank one', () => {
    const rows = parseProviderList([
      {
        id: 'llamacpp',
        display_name: 'llama.cpp',
        supports_model_listing: true,
        available: true,
        context_overflow_hint: 'Raise --ctx-size on the server.',
      },
      { id: 'openai', display_name: 'OpenAI', context_overflow_hint: '  ' },
      { id: 'anthropic', display_name: 'Anthropic' },
      'not a row',
    ])
    expect(rows.map((r) => r.id)).toEqual(['llamacpp', 'openai', 'anthropic'])
    expect(rows[0].context_overflow_hint).toBe(
      'Raise --ctx-size on the server.',
    )
    expect(rows[1].context_overflow_hint).toBeUndefined()
    expect(rows[2].context_overflow_hint).toBeUndefined()
  })

  it('remembers the last list for message-level lookups', () => {
    rememberProviderList(
      parseProviderList([{ id: 'llamacpp', context_overflow_hint: 'hint' }]),
    )
    expect(rememberedProvider('llamacpp')?.context_overflow_hint).toBe('hint')
    expect(rememberedProvider('openai')).toBeUndefined()
    rememberProviderList([])
    expect(rememberedProvider('llamacpp')).toBeUndefined()
  })
})

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
        credential_env_var: 'ANTHROPIC_API_KEY',
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

  it('trusts a listed default from a provider that owns its authentication', () => {
    // openai-codex is signed in through ~/.codex/auth.json; the router has no
    // key for it and reports configured: false, yet its catalog is live.
    const providers = [
      provider('openai-codex', {
        default_model: 'codex/gpt-6-luna',
        configured: false,
      }),
      provider('openai', {
        default_model: 'gpt-6.1-sol',
        credential_env_var: 'OPENAI_API_KEY',
        configured: false,
      }),
    ]
    const keys = new Set([
      'openai-codex::codex/gpt-5.6-luna',
      'openai-codex::codex/gpt-6-luna',
      'openai::gpt-6.1-sol',
    ])
    expect(preferredStartingModel(providers, keys)).toBe(
      'openai-codex::codex/gpt-6-luna',
    )
  })
})

describe('catalogKeysInRouterOrder', () => {
  it("keeps the router's order instead of sorting by name", () => {
    const rows = [
      {
        provider: 'claude-code',
        id: 'claude-code/claude-sonnet-5-5',
        display_name: 'Claude Sonnet 5.5',
      },
      {
        provider: 'claude-code',
        id: 'claude-code/claude-fable-5',
        display_name: 'Claude Fable 5',
      },
    ] as CatalogModelRow[]
    expect(catalogKeysInRouterOrder(rows)).toEqual([
      'claude-code::claude-code/claude-sonnet-5-5',
      'claude-code::claude-code/claude-fable-5',
    ])
    // The picker still lists them alphabetically.
    expect(catalogRowsToModelOptions(rows).map((o) => o.id)[0]).toBe(
      'claude-code::claude-code/claude-fable-5',
    )
  })
})
