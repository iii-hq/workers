import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { discoverAndRegister } from '../../src/provider-anthropic/discover.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { WorkerConfig } from '../../src/provider-anthropic/config.js';

const WORKER: WorkerConfig = {
  default_api_url: 'https://api.anthropic.com/v1/messages',
  default_max_tokens: 8192,
};

const { resolveProviderMock } = vi.hoisted(() => ({
  resolveProviderMock: vi.fn(),
}));

vi.mock('../../src/runtime/provider-resolve.js', () => ({
  resolveProvider: resolveProviderMock,
}));

function makeIii() {
  const trigger = vi.fn().mockResolvedValue({ ids: [], count: 0 });
  return { iii: { trigger } as unknown as ISdk, trigger };
}

function byFn(trigger: ReturnType<typeof vi.fn>, id: string) {
  return (trigger.mock.calls as Array<[{ function_id: string; payload: unknown }]>).filter(
    (c) => c[0].function_id === id,
  );
}

describe('discoverAndRegister', () => {
  beforeEach(() => {
    resolveProviderMock.mockReset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('reconciles empty catalog when there is no credential', async () => {
    resolveProviderMock.mockResolvedValue({
      configured: false,
      credential: null,
      api_url: null,
      max_tokens: null,
      source: null,
    });
    const { iii, trigger } = makeIii();

    const out = await discoverAndRegister(iii, WORKER);
    expect(out).toEqual([]);
    expect(byFn(trigger, 'models::reconcile')).toHaveLength(1);
    expect(byFn(trigger, 'models::reconcile')[0][0].payload).toEqual({
      provider: 'anthropic',
      models: [],
    });
  });

  it('reconciles empty catalog on upstream 401 (invalid api key)', async () => {
    resolveProviderMock.mockResolvedValue({
      configured: true,
      credential: { type: 'api_key', key: 'sk-bad' },
      api_url: WORKER.default_api_url,
      max_tokens: 8192,
      source: 'stored',
    });
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: 'invalid' }), { status: 401 }),
    ) as typeof globalThis.fetch;

    const { iii, trigger } = makeIii();
    const out = await discoverAndRegister(iii, WORKER);
    expect(out).toEqual([]);
    expect(byFn(trigger, 'models::reconcile')).toHaveLength(1);
    expect(byFn(trigger, 'models::reconcile')[0][0].payload).toEqual({
      provider: 'anthropic',
      models: [],
    });
  });

  it('does not reconcile on transient upstream 503', async () => {
    resolveProviderMock.mockResolvedValue({
      configured: true,
      credential: { type: 'api_key', key: 'sk-ok' },
      api_url: WORKER.default_api_url,
      max_tokens: 8192,
      source: 'stored',
    });
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response('unavailable', { status: 503 }),
    ) as typeof globalThis.fetch;

    const { iii, trigger } = makeIii();
    const out = await discoverAndRegister(iii, WORKER);
    expect(out).toEqual([]);
    expect(byFn(trigger, 'models::reconcile')).toHaveLength(0);
  });

  it('reconciles discovered models in one call on success', async () => {
    resolveProviderMock.mockResolvedValue({
      configured: true,
      credential: { type: 'api_key', key: 'sk-good' },
      api_url: WORKER.default_api_url,
      max_tokens: 8192,
      source: 'stored',
    });
    globalThis.fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [{ id: 'claude-sonnet-4-20250514', display_name: 'Claude Sonnet 4' }],
        }),
        { status: 200 },
      ),
    ) as typeof globalThis.fetch;

    const { iii, trigger } = makeIii();
    const out = await discoverAndRegister(iii, WORKER);
    expect(out).toEqual(['claude-sonnet-4-20250514']);
    const reconcile = byFn(trigger, 'models::reconcile');
    expect(reconcile).toHaveLength(1);
    const payload = reconcile[0][0].payload as { provider: string; models: unknown[] };
    expect(payload.provider).toBe('anthropic');
    expect(payload.models).toHaveLength(1);
    expect((payload.models[0] as { id: string }).id).toBe('claude-sonnet-4-20250514');
  });
});
