import { describe, expect, it } from 'vitest'
import {
  apiKeyPlaceholder,
  humanizeProviderId,
  type LiveProvider,
  parseProviderList,
  providerBucket,
  providerCardIds,
  providerDisplayName,
  providerRuntimeStatus,
  visibleProviderIds,
} from './provider-cards'

const schema = {
  properties: {
    providers: {
      properties: {
        anthropic: {},
        openai: {},
      },
    },
  },
}

describe('providerCardIds', () => {
  it('shows providers from the schema when fresh-install config is null', () => {
    expect(providerCardIds(schema, null)).toEqual(['anthropic', 'openai'])
  })

  it('keeps configured providers that are no longer in the schema', () => {
    expect(
      providerCardIds(schema, {
        providers: { openai: { api_key: 'configured' }, custom: {} },
      }),
    ).toEqual(['anthropic', 'openai', 'custom'])
  })

  it('returns no cards only when neither source contains providers', () => {
    expect(providerCardIds(null, null)).toEqual([])
    expect(providerCardIds({ properties: {} }, {})).toEqual([])
  })
})

describe('parseProviderList', () => {
  it('reads id, display_name and available from router::provider::list', () => {
    expect(
      parseProviderList({
        providers: [
          {
            id: 'anthropic',
            display_name: 'Anthropic',
            available: true,
          },
          { id: 'openai', available: false },
        ],
      }),
    ).toEqual([
      { id: 'anthropic', display_name: 'Anthropic', available: true },
      { id: 'openai', display_name: 'openai', available: false },
    ])
  })

  it('treats a missing available flag as loaded (older routers)', () => {
    expect(parseProviderList({ providers: [{ id: 'kimi' }] })).toEqual([
      { id: 'kimi', display_name: 'kimi', available: true },
    ])
  })

  it('returns an empty list for a malformed payload', () => {
    expect(parseProviderList(null)).toEqual([])
    expect(parseProviderList({ providers: 'nope' })).toEqual([])
  })
})

describe('providerRuntimeStatus', () => {
  const schemaIds = ['anthropic', 'openai']
  const live: LiveProvider[] = [
    { id: 'anthropic', display_name: 'Anthropic', available: true },
    { id: 'openai', display_name: 'OpenAI', available: false },
  ]

  it('is unknown until the live list has been fetched', () => {
    expect(providerRuntimeStatus('anthropic', null, schemaIds)).toBe('unknown')
  })

  it('is loaded when the worker is in the list and available', () => {
    expect(providerRuntimeStatus('anthropic', live, schemaIds)).toBe('loaded')
  })

  it('is not-loaded when the worker is declared but unavailable', () => {
    expect(providerRuntimeStatus('openai', live, schemaIds)).toBe('not-loaded')
  })

  it('is not-loaded when the schema lists a provider the live list does not', () => {
    expect(providerRuntimeStatus('kimi', [], ['kimi'])).toBe('not-loaded')
  })

  it('is not-connected when the id exists only in saved config', () => {
    expect(providerRuntimeStatus('old-local', live, schemaIds)).toBe('not-connected')
  })
})

describe('providerBucket and visibleProviderIds', () => {
  const live: LiveProvider[] = [
    { id: 'anthropic', display_name: 'Anthropic', available: true },
    { id: 'openai', display_name: 'OpenAI', available: true },
    { id: 'kimi', display_name: 'Kimi', available: false },
  ]
  const schemaIds = ['anthropic', 'openai', 'kimi']
  const ids = ['kimi', 'anthropic', 'openai', 'old-local']
  const hasKey = (id: string) => id === 'anthropic' || id === 'old-local'

  it('buckets loaded+key as ready, loaded+empty as needs-key, anything else as not-loaded', () => {
    expect(providerBucket('loaded', true)).toBe('ready')
    expect(providerBucket('loaded', false)).toBe('needs-key')
    expect(providerBucket('not-loaded', true)).toBe('not-loaded')
    expect(providerBucket('not-connected', false)).toBe('not-loaded')
    expect(providerBucket('unknown', false)).toBe('needs-key')
  })

  it('sorts needs-key first, then ready, then not-loaded', () => {
    expect(
      visibleProviderIds({
        ids,
        schemaIds,
        live,
        hasKey,
        filter: 'all',
      }),
    ).toEqual(['openai', 'anthropic', 'kimi', 'old-local'])
  })

  it('filters to a single bucket', () => {
    expect(
      visibleProviderIds({
        ids,
        schemaIds,
        live,
        hasKey,
        filter: 'needs-key',
      }),
    ).toEqual(['openai'])
    expect(
      visibleProviderIds({
        ids,
        schemaIds,
        live,
        hasKey,
        filter: 'ready',
      }),
    ).toEqual(['anthropic'])
    expect(
      visibleProviderIds({
        ids,
        schemaIds,
        live,
        hasKey,
        filter: 'not-loaded',
      }),
    ).toEqual(['kimi', 'old-local'])
  })
})

describe('providerDisplayName', () => {
  it('prefers a live display_name that is not the raw id', () => {
    expect(providerDisplayName('anthropic', 'Anthropic')).toBe('Anthropic')
  })

  it('humanizes kebab ids when the live name is missing or equal to the id', () => {
    expect(humanizeProviderId('openai-codex')).toBe('OpenAI Codex')
    expect(humanizeProviderId('github-copilot')).toBe('GitHub Copilot')
    expect(humanizeProviderId('llamacpp')).toBe('llama.cpp')
    expect(providerDisplayName('openai-codex', 'openai-codex')).toBe('OpenAI Codex')
    expect(providerDisplayName('kimi')).toBe('Kimi')
  })
})

describe('apiKeyPlaceholder', () => {
  it('does not look like a saved env reference', () => {
    expect(apiKeyPlaceholder('anthropic')).toBe('env var, e.g. ${' + 'ANTHROPIC_API_KEY}')
    expect(apiKeyPlaceholder('anthropic').startsWith('${')).toBe(false)
  })
})
