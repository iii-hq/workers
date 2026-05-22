import { describe, expect, it } from 'vitest';
import { findEnvKeys, resolveCredential, statusFor } from '../../src/auth-credentials/resolve.js';
import { InMemoryStore } from '../../src/auth-credentials/store.js';

describe('resolveCredential', () => {
  it('prefers stored over env', async () => {
    const s = new InMemoryStore();
    await s.set('anthropic', { type: 'api_key', key: 'stored' });
    const r = await resolveCredential(s, 'anthropic', () => 'env-fallback');
    expect(r?.source).toBe('stored');
    expect(r?.credential).toEqual({ type: 'api_key', key: 'stored' });
  });

  it('falls back to env', async () => {
    const s = new InMemoryStore();
    const r = await resolveCredential(s, 'openai', (v) =>
      v === 'OPENAI_API_KEY' ? 'from-env' : undefined,
    );
    expect(r?.source).toBe('environment');
    expect(r?.credential).toEqual({ type: 'api_key', key: 'from-env' });
  });

  it('returns null for unknown provider', async () => {
    const s = new InMemoryStore();
    expect(await resolveCredential(s, 'nope', () => undefined)).toBeNull();
  });
});

describe('findEnvKeys', () => {
  it('skips empty values', () => {
    const matches = findEnvKeys((var_) => {
      if (var_ === 'ANTHROPIC_API_KEY') return 'sk-ant-actual';
      if (var_ === 'OPENAI_API_KEY') return '';
      return undefined;
    });
    expect(matches.length).toBe(1);
    expect(matches[0]?.provider).toBe('anthropic');
    expect(matches[0]?.key_prefix).toBe('sk-ant-a');
  });
});

describe('statusFor', () => {
  it('reports configured + label when resolved', () => {
    const st = statusFor({
      credential: { type: 'api_key', key: 'sk-anthxx' },
      source: 'stored',
    });
    expect(st.configured).toBe(true);
    expect(st.source).toBe('stored');
    expect(st.label?.startsWith('api-key:')).toBe(true);
  });

  it('reports unconfigured when null', () => {
    expect(statusFor(null).configured).toBe(false);
  });
});
