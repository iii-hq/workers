import { describe, expect, it } from 'vitest';
import { loadConfig } from '../src/config.js';
import { registerOpencodeConfig } from '../src/configuration.js';
import { fakeIii } from './_helpers/fake-iii.js';

describe('configuration worker integration', () => {
  it('registerOpencodeConfig registers the schema with the seed as initial_value', async () => {
    const fake = fakeIii();
    const cfg = await loadConfig('/nonexistent/config.yaml');
    await registerOpencodeConfig(fake.iii, cfg);
    const reg = fake.calls.find((c) => c.function_id === 'configuration::register');
    expect(reg).toBeDefined();
    const payload = reg?.payload as {
      id?: string;
      schema?: { $schema?: unknown };
      initial_value?: Record<string, unknown>;
    };
    expect(payload.id).toBe('opencode');
    expect(payload.schema).toBeDefined();
    expect((payload.schema as { $schema?: unknown }).$schema).toBeUndefined();
    expect(payload.initial_value).not.toHaveProperty('engine_url');
    expect(payload.initial_value?.iii_context).toBe(true);
  });
});
