import { describe, expect, it } from 'vitest'
import {
  ACTIVE_PROVIDERS,
  type ActiveProvider,
  PROVIDER_DEFAULTS,
  PROVIDERS,
} from '@/components/providers/provider-registry'
import {
  MAX_TOKENS_LIMIT,
  normalizeErrorMessage,
  validateApiUrl,
  validateMaxTokens,
} from '@/lib/providers'

describe('provider-registry', () => {
  it('partitions into 5 active and 19 env-var-only providers', () => {
    const active = PROVIDERS.filter((p) => p.isActive)
    const envOnly = PROVIDERS.filter((p) => !p.isActive)
    expect(active).toHaveLength(5)
    expect(envOnly).toHaveLength(19)
    expect(active.length + envOnly.length).toBe(PROVIDERS.length)
  })

  it('active providers match ACTIVE_PROVIDERS list', () => {
    const activeIds = PROVIDERS.filter((p) => p.isActive).map((p) => p.id)
    expect(activeIds).toEqual([...ACTIVE_PROVIDERS])
  })

  it('anthropic entry has correct env var', () => {
    const entry = PROVIDERS.find((p) => p.id === 'anthropic')
    expect(entry).toBeDefined()
    expect(entry?.envVar).toBe('ANTHROPIC_API_KEY')
    expect(entry?.isActive).toBe(true)
  })

  it('openrouter entry is env-var-only', () => {
    const entry = PROVIDERS.find((p) => p.id === 'openrouter')
    expect(entry).toBeDefined()
    expect(entry?.envVar).toBe('OPENROUTER_API_KEY')
    expect(entry?.isActive).toBe(false)
  })

  it('PROVIDER_DEFAULTS has entries for all active providers', () => {
    for (const id of ACTIVE_PROVIDERS) {
      const defaults = PROVIDER_DEFAULTS[id as ActiveProvider]
      expect(defaults).toBeDefined()
      expect(typeof defaults.default_api_url).toBe('string')
      expect(defaults.default_api_url.length).toBeGreaterThan(0)
      expect(typeof defaults.default_max_tokens).toBe('number')
      expect(defaults.default_max_tokens).toBeGreaterThan(0)
    }
  })

  it('default URLs are valid', () => {
    for (const id of ACTIVE_PROVIDERS) {
      const { default_api_url } = PROVIDER_DEFAULTS[id as ActiveProvider]
      expect(() => new URL(default_api_url)).not.toThrow()
    }
  })
})

describe('normalizeErrorMessage', () => {
  it('returns "unknown error" for null/undefined', () => {
    expect(normalizeErrorMessage(null)).toBe('unknown error')
    expect(normalizeErrorMessage(undefined)).toBe('unknown error')
  })

  it('extracts message from Error instances and strips the "Error:" prefix', () => {
    expect(normalizeErrorMessage(new Error('boom'))).toBe('boom')
  })

  it('strips the @fn(<id>) prefix that workers prepend to bus errors', () => {
    expect(
      normalizeErrorMessage(new Error('@fn(auth::set_token) bad payload')),
    ).toBe('bad payload')
  })

  it('renders plain-object bus rejections (the iii-browser-sdk shape) as "<code>: <message>"', () => {
    // Regression: this object shape used to stringify to "[object Object]",
    // which then lowercased to "[object object]" and hid every real bus error.
    expect(
      normalizeErrorMessage({
        code: 'function_not_found',
        message: 'provider_config::get not registered',
      }),
    ).toBe('function_not_found: provider_config::get not registered')
  })

  it('renders bus rejections with only a message field', () => {
    expect(normalizeErrorMessage({ message: 'invalid url' })).toBe(
      'invalid url',
    )
  })

  it('renders bus rejections with only a code field', () => {
    expect(normalizeErrorMessage({ code: 'timeout' })).toBe('timeout')
  })

  it('falls back to the .error field when there is no .message', () => {
    expect(normalizeErrorMessage({ error: 'bad input' })).toBe('bad input')
  })

  it('falls back to String() for unrecognised object shapes', () => {
    // Keeps the old behaviour as the floor: we never throw on weird inputs.
    expect(normalizeErrorMessage({ unrelated: true })).toBe('[object object]')
  })
})

describe('validateApiUrl', () => {
  it('accepts http URLs', () => {
    expect(validateApiUrl('http://localhost:1234/v1/chat/completions')).toEqual(
      {
        ok: true,
        value: 'http://localhost:1234/v1/chat/completions',
      },
    )
  })

  it('accepts https URLs', () => {
    const r = validateApiUrl('https://api.openai.com/v1/chat/completions')
    expect(r.ok).toBe(true)
  })

  it('trims leading and trailing whitespace before validating', () => {
    expect(validateApiUrl('  http://localhost:1234  ')).toEqual({
      ok: true,
      value: 'http://localhost:1234',
    })
  })

  it('rejects unparseable input with a hint mentioning http(s)://', () => {
    const r = validateApiUrl('not a url')
    expect(r.ok).toBe(false)
    if (!r.ok) {
      expect(r.error).toMatch(/http:\/\/ or https:\/\//)
    }
  })

  it('rejects non-http(s) schemes naming the offending protocol', () => {
    const r = validateApiUrl('file:///etc/passwd')
    expect(r.ok).toBe(false)
    if (!r.ok) {
      expect(r.error).toMatch(/file/)
    }
  })
})

describe('validateMaxTokens', () => {
  it('accepts a positive integer within the harness ceiling', () => {
    expect(validateMaxTokens('8192')).toEqual({ ok: true, value: 8192 })
  })

  it('rejects decimals', () => {
    const r = validateMaxTokens('8192.5')
    expect(r.ok).toBe(false)
    if (!r.ok) {
      expect(r.error).toMatch(/whole number/)
    }
  })

  it('rejects zero and negative numbers', () => {
    expect(validateMaxTokens('0').ok).toBe(false)
    expect(validateMaxTokens('-1').ok).toBe(false)
  })

  it('rejects values above the harness ceiling', () => {
    expect(validateMaxTokens(String(MAX_TOKENS_LIMIT + 1)).ok).toBe(false)
  })

  it('accepts the harness ceiling exactly', () => {
    expect(validateMaxTokens(String(MAX_TOKENS_LIMIT))).toEqual({
      ok: true,
      value: MAX_TOKENS_LIMIT,
    })
  })

  it('rejects non-numeric input', () => {
    expect(validateMaxTokens('lots').ok).toBe(false)
  })
})
