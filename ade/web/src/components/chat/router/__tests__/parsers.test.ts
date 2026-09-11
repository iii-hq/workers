import { describe, expect, it } from 'vitest'
import {
  formatTokens,
  isRouterFunction,
  modelsListResponseSchema,
  ROUTER_FUNCTION_IDS,
  safeParseResponse,
} from '../parsers'

describe('isRouterFunction', () => {
  it('matches the known set and rejects others', () => {
    for (const id of ROUTER_FUNCTION_IDS)
      expect(isRouterFunction(id)).toBe(true)
    expect(isRouterFunction('router::route')).toBe(false)
    expect(isRouterFunction('workflow::status')).toBe(false)
  })
})

describe('formatTokens', () => {
  it('compacts millions and thousands, leaves small counts', () => {
    expect(formatTokens(1_000_000)).toBe('1M')
    expect(formatTokens(200_000)).toBe('200k')
    expect(formatTokens(128_000)).toBe('128k')
    expect(formatTokens(4096)).toBe('4.1k')
    expect(formatTokens(512)).toBe('512')
    expect(formatTokens(0)).toBeNull()
    expect(formatTokens(undefined)).toBeNull()
  })
})

describe('modelsListResponseSchema', () => {
  it('parses a harness-enveloped catalog with extra/unknown fields', () => {
    const enveloped = {
      content: [{ type: 'text', text: '{}' }],
      details: {
        models: [
          {
            id: 'claude-fable-5',
            provider: 'anthropic',
            display_name: 'Claude Fable 5',
            context_window: 1_000_000,
            max_output_tokens: 128_000,
            supports_thinking: true,
            supports_structured_output: false,
            pricing: {
              input: 10,
              output: 50,
              cache_read: 1,
              cache_write: 12.5,
            },
            // forward-compat: an unknown flag must not break the parse
            supports_future_thing: true,
          },
        ],
      },
      terminate: true,
    }
    const resp = safeParseResponse(modelsListResponseSchema, enveloped)
    expect(resp?.models).toHaveLength(1)
    expect(resp?.models[0].id).toBe('claude-fable-5')
    expect(resp?.models[0].pricing?.output).toBe(50)
  })

  it('defaults models to an empty array', () => {
    const resp = safeParseResponse(modelsListResponseSchema, {
      details: {},
    })
    expect(resp?.models).toEqual([])
  })
})
