import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Model } from '../../src/models-catalog/types.js';
import {
  _resetProviderResolveCacheForTests,
  buildConfig,
  invalidateProviderResolveCache,
} from '../../src/provider-anthropic/auth.js';
import type { WorkerConfig } from '../../src/provider-anthropic/config.js';
import { streamAnthropic } from '../../src/provider-anthropic/stream.js';
import { buildThinkingConfig } from '../../src/provider-anthropic/thinking.js';
import type { AnthropicConfig } from '../../src/provider-anthropic/types.js';
import type { ISdk } from '../../src/runtime/iii.js';

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

function resolved(max_tokens: number | null) {
  return {
    configured: true,
    credential: { type: 'api_key', key: 'sk-test' },
    api_url: null,
    max_tokens,
    source: 'stored',
  };
}

function iiiWithCatalog(entry: unknown, opts: { throws?: boolean } = {}): ISdk {
  const trigger = vi.fn().mockImplementation(async (req: { function_id: string }) => {
    if (req.function_id === 'models::get') {
      if (opts.throws) throw new Error('bus timeout');
      return entry;
    }
    return null;
  });
  return { trigger } as unknown as ISdk;
}

function iiiCountingCatalog(entry: unknown): { iii: ISdk; modelsGetCalls: () => number } {
  let n = 0;
  const trigger = vi.fn().mockImplementation(async (req: { function_id: string }) => {
    if (req.function_id === 'models::get') {
      n++;
      return entry;
    }
    return null;
  });
  return { iii: { trigger } as unknown as ISdk, modelsGetCalls: () => n };
}

const THINKING_MODEL: Model = {
  id: 'claude-sonnet-4-6',
  provider: 'anthropic',
  api: 'anthropic-messages',
  display_name: 'Claude Sonnet 4.6',
  context_window: 1_000_000,
  max_output_tokens: 64_000,
  supports_thinking: true,
  supports_xhigh: true,
  thinking_budgets: { minimal: 2_000, low: 4_000, medium: 8_000, high: 16_000 },
};

describe('buildConfig max_tokens resolution', () => {
  beforeEach(() => {
    resolveProviderMock.mockReset();
  });

  it('defaults to min(catalog max, 32k) — 64k model clamps to 32k', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(cfg.max_tokens).toBe(32_000);
    expect(cfg.catalog?.max_output_tokens).toBe(64_000);
  });

  it('uses the catalog max when below the cap', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const iii = iiiWithCatalog({ id: 'claude-haiku-4-5', max_output_tokens: 16_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-haiku-4-5');
    expect(cfg.max_tokens).toBe(16_000);
  });

  it('registry override wins below the model ceiling', async () => {
    resolveProviderMock.mockResolvedValue(resolved(50_000));
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(cfg.max_tokens).toBe(50_000);
  });

  it('registry override is clamped to the model ceiling', async () => {
    resolveProviderMock.mockResolvedValue(resolved(100_000));
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(cfg.max_tokens).toBe(64_000);
  });

  it('falls back to the registry/worker default when the catalog lookup fails', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const cfg = await buildConfig(iiiWithCatalog(null, { throws: true }), WORKER, 'claude-x');
    expect(cfg.max_tokens).toBe(WORKER.default_max_tokens);
    expect(cfg.catalog).toBeUndefined();
  });

  it('falls back to the worker default when the model is unknown', async () => {
    resolveProviderMock.mockResolvedValue(resolved(null));
    const cfg = await buildConfig(iiiWithCatalog(null), WORKER, 'claude-unknown');
    expect(cfg.max_tokens).toBe(WORKER.default_max_tokens);
  });

  it('throws without a credential', async () => {
    resolveProviderMock.mockResolvedValue({
      configured: false,
      credential: null,
      api_url: null,
      max_tokens: null,
      source: null,
    });
    await expect(buildConfig(iiiWithCatalog(null), WORKER, 'claude-x')).rejects.toThrow(
      /no credential/,
    );
  });
});

describe('buildConfig pre-resolved model threading', () => {
  beforeEach(() => {
    resolveProviderMock.mockReset();
    resolveProviderMock.mockResolvedValue(resolved(null));
  });

  it('uses the pre-resolved model and skips models::get', async () => {
    const { iii, modelsGetCalls } = iiiCountingCatalog({ id: 'nope', max_output_tokens: 1 });

    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6', THINKING_MODEL);

    expect(modelsGetCalls()).toBe(0);
    expect(cfg.catalog).toEqual(THINKING_MODEL);
    expect(cfg.max_tokens).toBe(32_000);
  });

  it('fetches models::get when no pre-resolved model is threaded', async () => {
    const { iii, modelsGetCalls } = iiiCountingCatalog({
      id: 'claude-sonnet-4-6',
      max_output_tokens: 64_000,
    });

    const cfg = await buildConfig(iii, WORKER, 'claude-sonnet-4-6');

    expect(modelsGetCalls()).toBe(1);
    expect(cfg.catalog?.id).toBe('claude-sonnet-4-6');
  });

  it.each([
    'high',
    'xhigh',
  ])('produces a byte-identical thinking config (%s) with vs without the pre-resolved model', async (level) => {
    const fetched = await buildConfig(iiiWithCatalog(THINKING_MODEL), WORKER, 'claude-sonnet-4-6');
    const { iii } = iiiCountingCatalog({ id: 'nope' });
    const threaded = await buildConfig(iii, WORKER, 'claude-sonnet-4-6', THINKING_MODEL);

    const fetchedThinking = buildThinkingConfig(level, fetched.max_tokens, fetched.catalog);
    const threadedThinking = buildThinkingConfig(level, threaded.max_tokens, threaded.catalog);

    expect(threadedThinking).toEqual(fetchedThinking);
    expect(threadedThinking).toBeDefined();
  });
});

function anthropicCfg(): AnthropicConfig {
  return {
    credential_value: 'sk-test',
    model: 'claude-sonnet-4-6',
    max_tokens: 32_000,
    api_url: 'https://api.example/v1/messages',
    auth_mode: 'api_key',
  };
}

describe('buildConfig per-turn credential resolution cache', () => {
  beforeEach(() => {
    resolveProviderMock.mockReset();
    resolveProviderMock.mockResolvedValue(resolved(null));
    _resetProviderResolveCacheForTests();
  });

  it('resolves once per turn key and reuses it across streams', async () => {
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
    expect(resolveProviderMock).toHaveBeenCalledTimes(1);
  });

  it('re-resolves when the turn key changes (next user turn)', async () => {
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 200);
    expect(resolveProviderMock).toHaveBeenCalledTimes(2);
  });

  it('does not cache when no turn key is threaded', async () => {
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6');
    expect(resolveProviderMock).toHaveBeenCalledTimes(2);
  });

  it('invalidateProviderResolveCache forces a re-resolve on the same key', async () => {
    const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
    invalidateProviderResolveCache();
    await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
    expect(resolveProviderMock).toHaveBeenCalledTimes(2);
  });

  describe('401 invalidation via streamAnthropic', () => {
    afterEach(() => {
      vi.unstubAllGlobals();
    });

    it('a 401 stream drops the cache so the next turn re-resolves the credential', async () => {
      const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
      await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
      await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
      expect(resolveProviderMock).toHaveBeenCalledTimes(1);

      vi.stubGlobal(
        'fetch',
        vi.fn(async () => new Response('unauthorized', { status: 401 })),
      );
      for await (const _ev of streamAnthropic({
        cfg: anthropicCfg(),
        system_prompt: '',
        messages: [],
        tools: [],
      })) {
      }

      await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
      expect(resolveProviderMock).toHaveBeenCalledTimes(2);
    });

    it('a 200 stream leaves the cache intact', async () => {
      const iii = iiiWithCatalog({ id: 'claude-sonnet-4-6', max_output_tokens: 64_000 });
      await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);

      vi.stubGlobal(
        'fetch',
        vi.fn(async () => new Response('data: {"type":"message_stop"}\n\n', { status: 200 })),
      );
      for await (const _ev of streamAnthropic({
        cfg: anthropicCfg(),
        system_prompt: '',
        messages: [],
        tools: [],
      })) {
      }

      await buildConfig(iii, WORKER, 'claude-sonnet-4-6', undefined, 100);
      expect(resolveProviderMock).toHaveBeenCalledTimes(1);
    });
  });
});
