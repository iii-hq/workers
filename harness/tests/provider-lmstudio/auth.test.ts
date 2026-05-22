import { describe, expect, it, vi } from 'vitest';
import { buildConfig } from '../../src/provider-lmstudio/auth.js';
import type { WorkerConfig } from '../../src/provider-lmstudio/config.js';
import type { ISdk } from '../../src/runtime/iii.js';

const worker: WorkerConfig = {
  default_max_tokens: 8192,
  default_api_url: 'http://localhost:1234/v1/chat/completions',
};

/**
 * Narrow ISdk stub used only by `buildConfig` tests. By design this only
 * implements `trigger`/`registerFunction` — `buildConfig` doesn't touch
 * any other SDK surface. If `auth.ts` ever calls a second SDK method
 * (e.g. emitting a metric), update this helper rather than letting the
 * `as unknown` cast silently mask the missing method.
 */
function makeSdk(triggerImpl: (req: { function_id: string; payload: unknown }) => unknown): ISdk {
  return {
    trigger: vi.fn(triggerImpl),
    registerFunction: vi.fn(),
  } as unknown as ISdk;
}

describe('buildConfig (lmstudio)', () => {
  it('falls back to api_key="lm-studio" when no credential is stored', async () => {
    const sdk = makeSdk(() => null);
    const cfg = await buildConfig(sdk, worker, 'qwen/qwen3-4b-2507');
    expect(cfg.api_key).toBe('lm-studio');
    expect(cfg.url).toBe(worker.default_api_url);
    expect(cfg.provider_name).toBe('lmstudio');
    expect(cfg.model).toBe('qwen/qwen3-4b-2507');
    expect(cfg.max_tokens).toBe(worker.default_max_tokens);
  });

  it('falls back to api_key="lm-studio" when auth::get_token throws', async () => {
    const sdk = makeSdk(() => {
      throw new Error('auth-credentials worker unreachable');
    });
    const cfg = await buildConfig(sdk, worker, 'qwen/qwen3-4b-2507');
    expect(cfg.api_key).toBe('lm-studio');
  });

  it('falls back to api_key="lm-studio" when credential has empty key', async () => {
    const sdk = makeSdk(() => ({ type: 'api_key', key: '' }));
    const cfg = await buildConfig(sdk, worker, 'qwen/qwen3-4b-2507');
    expect(cfg.api_key).toBe('lm-studio');
  });

  it('honours a real API key when LMSTUDIO_API_KEY is set on an authenticated deployment', async () => {
    const sdk = makeSdk(() => ({ type: 'api_key', key: 'sk-real-key' }));
    const cfg = await buildConfig(sdk, worker, 'qwen/qwen3-4b-2507');
    expect(cfg.api_key).toBe('sk-real-key');
  });

  it('calls auth::get_token with provider="lmstudio"', async () => {
    const trigger = vi.fn().mockResolvedValue(null);
    const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
    await buildConfig(sdk, worker, 'qwen/qwen3-4b-2507');
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'auth::get_token',
        payload: { provider: 'lmstudio' },
      }),
    );
  });
});
