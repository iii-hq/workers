import type { IIIClient } from 'iii-sdk';
import { describe, expect, it, vi } from 'vitest';
import { loadConfig } from '../src/config.js';
import {
  fetchRuntime,
  observeConfigChanges,
  registerOpencodeConfig,
} from '../src/configuration.js';
import { fakeIii } from './_helpers/fake-iii.js';

describe('configuration worker integration', () => {
  it('registerOpencodeConfig atomically submits the schema and seed without a preliminary read', async () => {
    const fake = fakeIii();
    const cfg = await loadConfig('/nonexistent/config.yaml');
    await registerOpencodeConfig(fake.iii, cfg);
    expect(fake.calls.map((call) => call.function_id)).toEqual(['configuration::ensure']);
    const reg = fake.calls.find((c) => c.function_id === 'configuration::ensure');
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

  it('observeConfigChanges runs onChange once and observes configuration::on-updated', async () => {
    const fake = fakeIii();
    const onChange = vi.fn(async () => {});
    await observeConfigChanges(fake.iii, onChange);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(fake.registered.has('opencode::on-config-change')).toBe(true);
    expect(fake.observed).toEqual([
      {
        source: {
          function_id: 'configuration::on-updated',
          filters: {
            'input.id': { $eq: 'opencode' },
            'input.event_type': { $eq: 'configuration:updated' },
          },
          deliver: 'output',
        },
        target: 'opencode::on-config-change',
      },
    ]);
    await fake.registered.get('opencode::on-config-change')?.({});
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('fetchRuntime returns the live value, null when unset', async () => {
    let namespace: string | undefined;
    const withValue = {
      call: async (request: { namespace?: string }) => {
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
    } as unknown as IIIClient;
    await expect(fetchRuntime(withValue)).resolves.toMatchObject({ defaults: { model: 'm' } });
    expect(namespace).toBe('default');
    const empty = { call: async () => ({ value: null }) } as unknown as IIIClient;
    await expect(fetchRuntime(empty)).resolves.toBeNull();
  });
});
