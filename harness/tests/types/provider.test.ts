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
    // model_meta is an optional optimization: a partial catalog entry must fall
    // back per-field at the consumer, never reject the whole input.
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
});

describe('ProviderStreamOutputSchema', () => {
  it('parses ok-only payloads', () => {
    expect(ProviderStreamOutputSchema.parse({ ok: true }).ok).toBe(true);
  });
});
