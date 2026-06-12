import { describe, expect, it } from 'vitest';
import {
  ProviderStreamInputJsonSchema,
  ProviderStreamInputSchema,
  ProviderStreamOutputSchema,
} from '../../src/types/provider.js';

describe('ProviderStreamInputSchema', () => {
  it('accepts minimum required fields', () => {
    const ok = ProviderStreamInputSchema.parse({
      writer_ref: { channel_id: 'c', access_key: 'k', direction: 'write' },
      model: 'claude-3-5-sonnet',
      messages: [],
    });
    expect(ok.tools).toEqual([]);
  });

  it('rejects bad direction', () => {
    expect(() =>
      ProviderStreamInputSchema.parse({
        writer_ref: { channel_id: 'c', access_key: 'k', direction: 'bidir' },
        model: 'm',
        messages: [],
      }),
    ).toThrow();
  });

  it('exposes a JSON schema with the writer_ref and model fields', () => {
    const schema = ProviderStreamInputJsonSchema as Record<string, unknown>;
    expect(JSON.stringify(schema)).toContain('writer_ref');
    expect(JSON.stringify(schema)).toContain('model');
  });

  it('passes a sparse model_meta through instead of failing the stream', () => {
    const sparse = ProviderStreamInputSchema.parse({
      writer_ref: { channel_id: 'c', access_key: 'k', direction: 'write' },
      model: 'm',
      messages: [],
      model_meta: { id: 'm', provider: 'anthropic' },
    });
    expect(sparse.model_meta).toEqual({ id: 'm', provider: 'anthropic' });
  });

  it('coerces a non-object model_meta to absent', () => {
    const parsed = ProviderStreamInputSchema.parse({
      writer_ref: { channel_id: 'c', access_key: 'k', direction: 'write' },
      model: 'm',
      messages: [],
      model_meta: 'garbage',
    });
    expect(parsed.model_meta).toBeUndefined();
  });

  it('tolerates null options — a default router turn must never fail validation', () => {
    // The router omits absent options, but a null that slips through must
    // degrade to the defaults instead of erroring the whole stream.
    const parsed = ProviderStreamInputSchema.parse({
      writer_ref: { channel_id: 'c', access_key: 'k', direction: 'write' },
      model: 'm',
      messages: [],
      tools: null,
      thinking_level: null,
      resolution_key: null,
      max_output_tokens: null,
    });
    expect(parsed.tools).toEqual([]);
    expect(parsed.thinking_level).toBeUndefined();
    expect(parsed.resolution_key).toBeUndefined();
    expect(parsed.max_output_tokens).toBeUndefined();
  });

  it('accepts the router-built payload: resolved budget, string resolution key, options', () => {
    const parsed = ProviderStreamInputSchema.parse({
      writer_ref: { channel_id: 'c', access_key: 'k', direction: 'write' },
      model: 'm',
      messages: [],
      tools: [{ name: 't', description: 'd', parameters: {} }],
      thinking_level: 'high',
      max_output_tokens: 24_000,
      resolution_key: 'sess-1:1718000000000',
      response_format: { type: 'json' },
      provider_options: { beta: true },
    });
    expect(parsed.max_output_tokens).toBe(24_000);
    expect(parsed.resolution_key).toBe('sess-1:1718000000000');
    expect(parsed.response_format).toEqual({ type: 'json' });
    expect(parsed.provider_options).toEqual({ beta: true });
  });

  it('still accepts numeric resolution keys', () => {
    const parsed = ProviderStreamInputSchema.parse({
      writer_ref: { channel_id: 'c', access_key: 'k', direction: 'write' },
      model: 'm',
      messages: [],
      resolution_key: 1718000000000,
    });
    expect(parsed.resolution_key).toBe(1718000000000);
  });
});

describe('ProviderStreamOutputSchema', () => {
  it('parses ok-only payloads', () => {
    expect(ProviderStreamOutputSchema.parse({ ok: true }).ok).toBe(true);
  });
});
