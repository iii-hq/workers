import { describe, expect, it, vi } from 'vitest';
import {
  buildAuthHeaders,
  buildConfig,
  isLoopbackUrl,
  selectAuthKey,
} from '../../src/provider-lmstudio/auth.js';
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

describe('isLoopbackUrl', () => {
  it('accepts canonical loopback hosts', () => {
    expect(isLoopbackUrl('http://localhost:1234/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('http://127.0.0.1:1234/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('http://127.0.0.5/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('http://[::1]/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('https://api.localhost/v1/chat/completions')).toBe(true);
  });

  it('rejects non-loopback hosts', () => {
    expect(isLoopbackUrl('http://example.com/v1/chat/completions')).toBe(false);
    expect(isLoopbackUrl('http://192.168.1.10/v1/chat/completions')).toBe(false);
    expect(isLoopbackUrl('https://my-tunnel.ngrok.io/v1/chat/completions')).toBe(false);
  });

  it('rejects malformed URLs (fail closed)', () => {
    expect(isLoopbackUrl('not-a-url')).toBe(false);
    expect(isLoopbackUrl('')).toBe(false);
  });
});

describe('selectAuthKey', () => {
  it('returns the explicit key when one is configured', () => {
    expect(selectAuthKey({ type: 'api_key', key: 'sk-explicit' }, 'http://localhost:1234/')).toBe(
      'sk-explicit',
    );
    expect(selectAuthKey({ type: 'api_key', key: 'sk-explicit' }, 'https://remote.example/')).toBe(
      'sk-explicit',
    );
  });

  it('returns the fallback for loopback when no credential is configured', () => {
    expect(selectAuthKey(null, 'http://localhost:1234/v1/chat/completions')).toBe('lm-studio');
    expect(selectAuthKey(null, 'http://127.0.0.1:1234/v1/chat/completions')).toBe('lm-studio');
  });

  it('returns null (omit Authorization) for non-loopback without a credential', () => {
    expect(selectAuthKey(null, 'https://my-tunnel.ngrok.io/v1/chat/completions')).toBeNull();
    expect(selectAuthKey(null, 'https://api.example.com/v1/chat/completions')).toBeNull();
  });
});

describe('buildAuthHeaders', () => {
  function sdkWith(cred: unknown): ISdk {
    return {
      trigger: vi.fn().mockResolvedValue(cred),
      registerFunction: vi.fn(),
    } as unknown as ISdk;
  }

  it('emits Authorization for loopback even without a credential', async () => {
    const headers = await buildAuthHeaders(
      sdkWith(null),
      'http://localhost:1234/v1/chat/completions',
    );
    expect(headers.Authorization).toBe('Bearer lm-studio');
    expect(headers['content-type']).toBe('application/json');
  });

  it('OMITS Authorization for non-loopback without a credential', async () => {
    const headers = await buildAuthHeaders(
      sdkWith(null),
      'https://my-tunnel.ngrok.io/v1/chat/completions',
    );
    expect('Authorization' in headers).toBe(false);
    expect(headers['content-type']).toBe('application/json');
  });

  it('emits an explicit key on non-loopback when configured', async () => {
    const headers = await buildAuthHeaders(
      sdkWith({ type: 'api_key', key: 'sk-real' }),
      'https://my-tunnel.ngrok.io/v1/chat/completions',
    );
    expect(headers.Authorization).toBe('Bearer sk-real');
  });
});
