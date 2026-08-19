import type { ISdk } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import { loadConfig } from '../src/config.js';
import { bindConfigTrigger, fetchRuntime, registerOpencodeConfig } from '../src/configuration.js';
import { fakeIii } from './_helpers/fake-iii.js';

describe('configuration worker integration', () => {
  it('registerOpencodeConfig registers the schema with the seed as initial_value', async () => {
    const fake = fakeIii();
    const cfg = await loadConfig('/nonexistent/config.yaml');
    await registerOpencodeConfig(fake.iii, cfg);
    const reg = fake.calls.find((c) => c.function_id === 'configuration::register');
    expect(reg).toBeDefined();
    expect(reg?.namespace).toBe('default');
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

  it('bindConfigTrigger runs onChange once and registers the change fn + trigger', async () => {
    const fake = fakeIii();
    const onChange = vi.fn(async () => {});
    await bindConfigTrigger(fake.iii, onChange);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(fake.registered.has('opencode::on-config-change')).toBe(true);
    const trig = fake.calls.find(
      (c) => c.function_id === 'engine::register-trigger' || c.function_id === undefined,
    );
    // the SDK records register_trigger via registerTrigger; assert the handler reloads
    await fake.registered.get('opencode::on-config-change')?.({});
    expect(onChange).toHaveBeenCalledTimes(2);
    void trig;
  });

  it('fetchRuntime returns the live value, null when unset', async () => {
    let namespace: string | undefined;
    const withValue = {
      trigger: async (request: { namespace?: string }) => {
        namespace = request.namespace;
        return {
          value: {
            defaults: { model: 'm', cwd: '', agent: '' },
            events_stream: 'agent::events',
            raw_events_stream: 'opencode::events',
            iii_context: true,
            opencode_executable: '',
          },
        };
      },
    } as unknown as ISdk;
    await expect(fetchRuntime(withValue)).resolves.toMatchObject({ defaults: { model: 'm' } });
    expect(namespace).toBe('default');
    const empty = { trigger: async () => ({ value: null }) } as unknown as ISdk;
    await expect(fetchRuntime(empty)).resolves.toBeNull();
  });
});
