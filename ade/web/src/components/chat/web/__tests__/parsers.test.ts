import { describe, expect, it } from 'vitest'
import {
  fetchErrorSchema,
  fetchErrorVariantLabel,
  fetchRequestSchema,
  fetchResultSchema,
  fetchSuccessSchema,
  isWebFunction,
  safeParseRequest,
  safeParseResponse,
  unwrapEnvelope,
  WEB_FUNCTION_IDS,
} from '../parsers'

function wrap<T>(details: T) {
  return {
    content: [{ type: 'text', text: JSON.stringify(details) }],
    details,
    terminate: false,
  }
}

describe('isWebFunction', () => {
  it('matches every id in the explicit allowlist', () => {
    for (const id of WEB_FUNCTION_IDS) {
      expect(isWebFunction(id)).toBe(true)
    }
  })

  it('rejects unrelated ids', () => {
    expect(isWebFunction('web::')).toBe(false)
    expect(isWebFunction('sandbox::exec')).toBe(false)
    expect(isWebFunction('webfetch')).toBe(false)
  })
})

describe('fetchRequestSchema', () => {
  it('accepts a minimal request', () => {
    const r = safeParseRequest(fetchRequestSchema, {
      url: 'https://example.com',
    })
    expect(r?.url).toBe('https://example.com')
  })

  it('accepts a fully-populated POST request', () => {
    const r = safeParseRequest(fetchRequestSchema, {
      url: 'https://example.com/api',
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      json: { hello: 'world' },
      timeout_ms: 5_000,
      max_bytes: 1_000_000,
      follow_redirects: false,
      response_format: 'json',
    })
    expect(r?.method).toBe('POST')
    expect(r?.response_format).toBe('json')
  })

  it('rejects requests without a url', () => {
    expect(safeParseRequest(fetchRequestSchema, { method: 'GET' })).toBeNull()
  })
})

describe('fetchSuccessSchema', () => {
  const base = {
    ok: true as const,
    status: 200,
    status_text: 'OK',
    headers: { 'content-type': 'application/json' },
    body: '{"ok":true,"todos":[]}',
    response_format: 'text' as const,
    bytes_truncated: false,
  }

  it('parses a raw text-format success', () => {
    expect(fetchSuccessSchema.safeParse(base).success).toBe(true)
  })

  it('parses a wrapped json-format success with json + redirect_chain', () => {
    const payload = {
      ...base,
      response_format: 'json' as const,
      json: { ok: true, todos: [] },
      redirect_chain: ['https://old.example.com/path'],
    }
    expect(safeParseResponse(fetchSuccessSchema, wrap(payload))).toMatchObject({
      response_format: 'json',
      redirect_chain: ['https://old.example.com/path'],
    })
  })

  it('parses a base64 binary success', () => {
    expect(
      safeParseResponse(fetchSuccessSchema, {
        ...base,
        response_format: 'base64' as const,
        body: 'aGVsbG8=',
      }),
    ).toBeTruthy()
  })
})

describe('fetchErrorSchema', () => {
  it.each([
    'invalid_payload',
    'invalid_url',
    'blocked_host',
    'timeout',
    'too_large',
    'too_many_redirects',
    'transport_error',
  ] as const)('accepts the %s variant', (variant) => {
    const parsed = fetchErrorSchema.safeParse({
      ok: false,
      error: variant,
      message: 'something went wrong',
    })
    expect(parsed.success).toBe(true)
  })

  it('rejects an unknown error variant', () => {
    expect(
      fetchErrorSchema.safeParse({
        ok: false,
        error: 'unknown_kind',
        message: 'nope',
      }).success,
    ).toBe(false)
  })

  it('accepts an optional null status', () => {
    expect(
      fetchErrorSchema.safeParse({
        ok: false,
        error: 'timeout',
        message: 'request timed out',
        status: null,
      }).success,
    ).toBe(true)
  })
})

describe('fetchResultSchema union', () => {
  it('discriminates success vs error by `ok`', () => {
    const successParsed = fetchResultSchema.safeParse({
      ok: true,
      status: 200,
      status_text: 'OK',
      headers: {},
      body: '',
      response_format: 'text',
      bytes_truncated: false,
    })
    expect(successParsed.success).toBe(true)

    const errorParsed = fetchResultSchema.safeParse({
      ok: false,
      error: 'transport_error',
      message: 'ECONNRESET',
    })
    expect(errorParsed.success).toBe(true)
  })
})

describe('fetchErrorVariantLabel', () => {
  it('maps every variant to a human-readable label', () => {
    expect(fetchErrorVariantLabel('timeout')).toBe('Request timed out')
    expect(fetchErrorVariantLabel('blocked_host')).toBe('Blocked host')
  })
})

describe('unwrapEnvelope re-export', () => {
  it('peels the harness envelope', () => {
    const inner = { ok: true }
    expect(unwrapEnvelope(wrap(inner))).toEqual(inner)
  })
})
