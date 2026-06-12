import { describe, expect, it, vi } from 'vitest';
import {
  buildAuthHeaders,
  buildConfig,
  isLoopbackUrl,
  selectAuthKey,
} from '../../src/provider-llamacpp/auth.js';
import type { WorkerConfig } from '../../src/provider-llamacpp/config.js';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  _seedRouterRegistrationTokenForTests,
  type Credential,
} from '../../src/runtime/provider-resolve.js';

_seedRouterRegistrationTokenForTests('llamacpp', 'tok-test');

const worker: WorkerConfig = {
  default_max_tokens: 8192,
  default_api_url: 'http://localhost:8080/v1/chat/completions',
};

/** Wrap a credential into the `router::provider::resolve` result shape. */
function resolveResult(cred: Credential | null) {
  return {
    configured: cred !== null,
    source: cred ? 'config' : 'none',
    credential: cred,
    api_url: null,
    max_tokens: null,
  };
}

function makeSdk(triggerImpl: (req: { function_id: string; payload: unknown }) => unknown): ISdk {
  return {
    trigger: vi.fn(triggerImpl),
    registerFunction: vi.fn(),
  } as unknown as ISdk;
}

describe('buildConfig (llamacpp)', () => {
  it('returns an empty api_key when no credential is stored (loopback)', async () => {
    const sdk = makeSdk(() => resolveResult(null));
    const cfg = await buildConfig(sdk, worker, 'Meta-Llama-3-8B');
    // Unlike LM Studio there is no fallback bearer; the empty string
    // is propagated and stream.ts omits the Authorization header.
    expect(cfg.api_key).toBe('');
    expect(cfg.url).toBe(worker.default_api_url);
    expect(cfg.provider_name).toBe('llamacpp');
    expect(cfg.model).toBe('Meta-Llama-3-8B');
    expect(cfg.max_tokens).toBe(worker.default_max_tokens);
  });

  it('returns an empty api_key when resolve throws', async () => {
    const sdk = makeSdk(() => {
      throw new Error('harness unreachable');
    });
    const cfg = await buildConfig(sdk, worker, 'Meta-Llama-3-8B');
    expect(cfg.api_key).toBe('');
  });

  it('honours a real API key when one is configured on an authenticated llama-server', async () => {
    const sdk = makeSdk(() => resolveResult({ type: 'api_key', key: 'sk-real-key' }));
    const cfg = await buildConfig(sdk, worker, 'Meta-Llama-3-8B');
    expect(cfg.api_key).toBe('sk-real-key');
  });

  it('resolves via router::provider::resolve with provider="llamacpp"', async () => {
    const trigger = vi.fn().mockResolvedValue(resolveResult(null));
    const sdk = { trigger, registerFunction: vi.fn() } as unknown as ISdk;
    await buildConfig(sdk, worker, 'Meta-Llama-3-8B');
    expect(trigger).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'router::provider::resolve',
        payload: { id: 'llamacpp', token: 'tok-test' },
      }),
    );
  });
});

describe('isLoopbackUrl', () => {
  it('accepts canonical loopback hosts', () => {
    expect(isLoopbackUrl('http://localhost:8080/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('http://127.0.0.1:8080/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('http://127.0.0.5/v1/chat/completions')).toBe(true);
    expect(isLoopbackUrl('http://[::1]/v1/chat/completions')).toBe(true);
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
    expect(selectAuthKey({ type: 'api_key', key: 'sk-explicit' }, 'http://localhost:8080/')).toBe(
      'sk-explicit',
    );
    expect(selectAuthKey({ type: 'api_key', key: 'sk-explicit' }, 'https://remote.example/')).toBe(
      'sk-explicit',
    );
  });

  it('returns null on loopback without a credential (no synthetic bearer)', () => {
    // Unlike LM Studio there is no documented default token, so we
    // simply omit Authorization on loopback when no key is configured.
    expect(selectAuthKey(null, 'http://localhost:8080/v1/chat/completions')).toBeNull();
    expect(selectAuthKey(null, 'http://127.0.0.1:8080/v1/chat/completions')).toBeNull();
  });

  it('returns null on non-loopback without a credential', () => {
    expect(selectAuthKey(null, 'https://my-tunnel.ngrok.io/v1/chat/completions')).toBeNull();
    expect(selectAuthKey(null, 'https://api.example.com/v1/chat/completions')).toBeNull();
  });
});

describe('buildAuthHeaders', () => {
  function sdkWith(cred: Credential | null): ISdk {
    return {
      trigger: vi.fn().mockResolvedValue(resolveResult(cred)),
      registerFunction: vi.fn(),
    } as unknown as ISdk;
  }

  it('OMITS Authorization on loopback without a credential', async () => {
    const headers = await buildAuthHeaders(
      sdkWith(null),
      'http://localhost:8080/v1/chat/completions',
    );
    expect('Authorization' in headers).toBe(false);
    expect(headers['content-type']).toBe('application/json');
  });

  it('OMITS Authorization on non-loopback without a credential', async () => {
    const headers = await buildAuthHeaders(
      sdkWith(null),
      'https://my-tunnel.ngrok.io/v1/chat/completions',
    );
    expect('Authorization' in headers).toBe(false);
  });

  it('emits an explicit key when configured', async () => {
    const headers = await buildAuthHeaders(
      sdkWith({ type: 'api_key', key: 'sk-real' }),
      'http://localhost:8080/v1/chat/completions',
    );
    expect(headers.Authorization).toBe('Bearer sk-real');
  });
});
