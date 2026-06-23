import { describe, expect, it } from 'vitest';
import { loadConfig, runtimeJsonSchema, toRuntime } from '../src/config.js';
import { registerPiConfig } from '../src/configuration.js';
import { fakeIii } from './_helpers/fake-iii.js';

describe('configuration worker integration', () => {
  it('runtime schema excludes the bootstrap engine_url', () => {
    const schema = runtimeJsonSchema() as { properties?: Record<string, unknown> };
    expect(schema.properties).toBeDefined();
    expect(schema.properties).not.toHaveProperty('engine_url');
    expect(schema.properties).toHaveProperty('defaults');
    expect(schema.properties).toHaveProperty('raw_events_stream');
  });

  it('toRuntime drops engine_url, keeps the rest', async () => {
    const cfg = await loadConfig('/nonexistent/config.yaml');
    const rt = toRuntime(cfg) as Record<string, unknown>;
    expect(rt).not.toHaveProperty('engine_url');
    expect(rt.raw_events_stream).toBe('pi::events');
    expect(rt.iii_context).toBe(true);
  });

  it('registerPiConfig registers the schema with the seed as initial_value', async () => {
    const fake = fakeIii();
    const cfg = await loadConfig('/nonexistent/config.yaml');
    await registerPiConfig(fake.iii, cfg);
    const reg = fake.calls.find((c) => c.function_id === 'configuration::register');
    expect(reg).toBeDefined();
    const payload = reg?.payload as {
      id?: string;
      schema?: unknown;
      initial_value?: Record<string, unknown>;
    };
    expect(payload.id).toBe('pi');
    expect(payload.schema).toBeDefined();
    expect(payload.initial_value).not.toHaveProperty('engine_url');
    expect(payload.initial_value?.iii_context).toBe(true);
  });
});
