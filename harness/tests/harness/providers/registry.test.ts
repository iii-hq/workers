import { describe, expect, it, vi } from 'vitest';
import { ProviderRegistry } from '../../../src/harness/providers/registry.js';
import type { ISdk } from '../../../src/runtime/iii.js';

/**
 * In-memory stand-in for the `configuration` worker: keeps one value + the
 * last-registered schema so we can assert the dynamic schema composition and
 * the resolve path without a live engine.
 */
function makeConfigSdk(initialValue: unknown) {
  const state: { value: unknown; schema: Record<string, unknown> | null } = {
    value: initialValue,
    schema: null,
  };
  const trigger = vi.fn(async (req: { function_id: string; payload: Record<string, unknown> }) => {
    switch (req.function_id) {
      case 'configuration::register': {
        state.schema = req.payload.schema as Record<string, unknown>;
        if (state.value === null && req.payload.initial_value !== undefined) {
          state.value = req.payload.initial_value;
        }
        return {};
      }
      case 'configuration::get':
        return { id: req.payload.id, value: state.value };
      case 'configuration::set':
        state.value = req.payload.value;
        return { new_value: state.value, old_value: null };
      default:
        return null;
    }
  });
  const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
  return { sdk, state };
}

describe('ProviderRegistry.init', () => {
  it('seeds the base harness entry (permissions + empty providers)', async () => {
    const { sdk, state } = makeConfigSdk(null);
    await new ProviderRegistry(sdk).init();
    expect(state.value).toEqual({
      permissions: { default_mode: 'manual' },
      providers: {},
    });
  });
});

describe('ProviderRegistry.declare', () => {
  it('composes a dynamic schema with one property per declared provider', async () => {
    const { sdk, state } = makeConfigSdk(null);
    const reg = new ProviderRegistry(sdk);
    await reg.declare({
      id: 'anthropic',
      defaults: { api_url: 'https://api.anthropic.com/v1/messages', max_tokens: 8192 },
    });
    await reg.declare({ id: 'openai', defaults: { max_tokens: 8192 } });

    const schema = state.schema as Record<string, Record<string, Record<string, unknown>>>;
    const providers = schema.properties.providers as Record<string, Record<string, unknown>>;
    expect(Object.keys(providers.properties)).toEqual(['anthropic', 'openai']);
    const permissions = schema.properties.permissions as Record<string, Record<string, unknown>>;
    const mode = (permissions.properties as Record<string, Record<string, unknown>>).default_mode;
    expect(mode.enum).toEqual(['manual', 'auto', 'full']);
  });
});

describe('ProviderRegistry.resolve', () => {
  it('returns the stored api key + settings (source=stored)', async () => {
    const { sdk } = makeConfigSdk({
      permissions: { default_mode: 'auto' },
      providers: {
        anthropic: { api_key: 'sk-abc', api_url: 'https://custom', max_tokens: 5000 },
      },
    });
    const reg = new ProviderRegistry(sdk);
    await reg.declare({
      id: 'anthropic',
      credential_env_var: 'ANTHROPIC_API_KEY',
      defaults: { api_url: 'https://default', max_tokens: 8192 },
    });
    const r = await reg.resolve('anthropic');
    expect(r.credential).toEqual({ type: 'api_key', key: 'sk-abc' });
    expect(r.source).toBe('stored');
    expect(r.api_url).toBe('https://custom');
    expect(r.max_tokens).toBe(5000);
    expect(r.configured).toBe(true);
  });

  it('falls back to the env var and declared defaults (source=environment)', async () => {
    process.env.III_TEST_PROVIDER_KEY = 'sk-env';
    try {
      const { sdk } = makeConfigSdk({ permissions: { default_mode: 'manual' }, providers: {} });
      const reg = new ProviderRegistry(sdk);
      await reg.declare({
        id: 'foo',
        credential_env_var: 'III_TEST_PROVIDER_KEY',
        defaults: { api_url: 'https://default', max_tokens: 1234 },
      });
      const r = await reg.resolve('foo');
      expect(r.credential).toEqual({ type: 'api_key', key: 'sk-env' });
      expect(r.source).toBe('environment');
      expect(r.api_url).toBe('https://default');
      // The declared default max_tokens is NOT seeded as an override —
      // seeding it would pin every request to 8192 and defeat the clamp.
      expect(r.max_tokens).toBeNull();
    } finally {
      process.env.III_TEST_PROVIDER_KEY = undefined;
    }
  });

  it('returns max_tokens only when the user actually configured it', async () => {
    const { sdk } = makeConfigSdk({
      permissions: { default_mode: 'manual' },
      providers: { anthropic: { api_key: 'sk-abc' } },
    });
    const reg = new ProviderRegistry(sdk);
    await reg.declare({
      id: 'anthropic',
      credential_env_var: 'ANTHROPIC_API_KEY',
      defaults: { api_url: 'https://default', max_tokens: 8192 },
    });
    const r = await reg.resolve('anthropic');
    expect(r.max_tokens).toBeNull();
  });

  it('reports unconfigured when neither stored nor env credential exists', async () => {
    const { sdk } = makeConfigSdk({ permissions: { default_mode: 'manual' }, providers: {} });
    const reg = new ProviderRegistry(sdk);
    await reg.declare({ id: 'bar', credential_env_var: 'III_TEST_UNSET_KEY', defaults: {} });
    const r = await reg.resolve('bar');
    expect(r.credential).toBeNull();
    expect(r.source).toBeNull();
    expect(r.configured).toBe(false);
  });
});
