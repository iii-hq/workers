import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { WebConfig } from '../../src/web/config.js';
import { register } from '../../src/web/handlers/fetch.js';

function fakeIii(): { iii: ISdk; registered: Map<string, (payload: unknown) => Promise<unknown>> } {
  const registered = new Map<string, (payload: unknown) => Promise<unknown>>();
  const iii = {
    registerFunction: (id: string, handler: (payload: unknown) => Promise<unknown>) => {
      registered.set(id, handler);
    },
  } as unknown as ISdk;
  return { iii, registered };
}

const cfg: WebConfig = {
  max_timeout_ms: 30_000,
  max_response_bytes: 5 * 1024 * 1024,
  max_redirects: 5,
  user_agent: 'test',
  allow_loopback: true,
};

describe('web::fetch handler', () => {
  it('registers under the right function id and exposes a request_format', () => {
    const { iii, registered } = fakeIii();
    const spy = vi.spyOn(iii, 'registerFunction');
    register(iii, cfg);
    expect(registered.has('web::fetch')).toBe(true);
    expect(spy).toHaveBeenCalledWith(
      'web::fetch',
      expect.any(Function),
      expect.objectContaining({
        description: expect.stringMatching(/INSTEAD of `shell::exec` with curl/),
        request_format: expect.any(Object),
      }),
    );
  });

  it('returns invalid_payload envelope (not throw) for missing url', async () => {
    const { iii, registered } = fakeIii();
    register(iii, cfg);
    const handler = registered.get('web::fetch');
    if (!handler) throw new Error('handler missing');
    const r = (await handler({})) as { ok: boolean; error?: string };
    expect(r.ok).toBe(false);
    expect(r.error).toBe('invalid_payload');
  });

  it('returns invalid_payload for a wrong-type body', async () => {
    const { iii, registered } = fakeIii();
    register(iii, cfg);
    const handler = registered.get('web::fetch');
    if (!handler) throw new Error('handler missing');
    const r = (await handler({ url: 'https://example.com/', body: 12345 })) as {
      ok: boolean;
      error?: string;
    };
    expect(r.ok).toBe(false);
    expect(r.error).toBe('invalid_payload');
  });

  it('returns invalid_url envelope for a bad scheme (validation passes, guard catches it)', async () => {
    const { iii, registered } = fakeIii();
    register(iii, cfg);
    const handler = registered.get('web::fetch');
    if (!handler) throw new Error('handler missing');
    const r = (await handler({ url: 'file:///etc/passwd' })) as { ok: boolean; error?: string };
    expect(r.ok).toBe(false);
    expect(r.error).toBe('invalid_url');
  });
});
