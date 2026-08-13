import { describe, expect, it } from 'vitest'
import { validateRouterConfig } from './validation'

describe('validateRouterConfig', () => {
  it('rejects invalid budgets, references, regexes, and urls', () => {
    const errors = validateRouterConfig(
      {
        default_provider: 'missing',
        settings: {
          stream_timeout_ms: 100,
          idle_timeout_ms: 101,
          retry_max: 11,
        },
        routing_heuristics: [{ pattern: '([', provider: 'missing' }],
        providers: { openai: { api_url: 'localhost:8080', max_tokens: 1.5 } },
      },
      ['openai'],
    )
    expect(errors.get('/default_provider')).toMatch(/not connected/)
    expect(errors.get('/settings/idle_timeout_ms')).toMatch(/stream timeout/)
    expect(errors.get('/settings/retry_max')).toMatch(/between 0 and 10/)
    expect(errors.get('/routing_heuristics/0/pattern')).toMatch(/regular expression/)
    expect(errors.get('/providers/openai/api_url')).toMatch(/absolute/)
    expect(errors.get('/providers/openai/max_tokens')).toMatch(/integer/)
  })

  it('accepts a valid configuration', () => {
    expect(
      validateRouterConfig(
        {
          default_provider: 'openai',
          settings: {
            stream_timeout_ms: 300_000,
            idle_timeout_ms: 120_000,
            retry_max: 2,
            output_token_max: 32_000,
          },
          routing_heuristics: [{ pattern: '^gpt-', provider: 'openai' }],
          providers: { openai: { api_url: 'https://api.openai.com/v1', max_tokens: 1 } },
        },
        ['openai'],
      ).size,
    ).toBe(0)
  })
})
