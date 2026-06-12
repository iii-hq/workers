import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  _resetRouterRegistrationForTests,
  _seedRouterRegistrationTokenForTests,
  registerWithRouter,
  resolveProviderViaRouter,
  routerRegistrationToken,
  subscribeRouterReady,
} from '../../src/runtime/provider-resolve.js';

type TriggerReq = { function_id: string; payload: Record<string, unknown> };

/**
 * Fake bus backing iii-state with a Map and `router::provider::register`
 * with a configurable handler.
 */
function makeIii(opts: {
  onRegister?: (payload: Record<string, unknown>) => unknown;
  onResolve?: (payload: Record<string, unknown>) => unknown;
  state?: Map<string, unknown>;
}) {
  const state = opts.state ?? new Map<string, unknown>();
  const key = (p: Record<string, unknown>) => `${p.scope}/${p.key}`;
  const trigger = vi.fn(async (req: TriggerReq) => {
    const { function_id, payload } = req;
    if (function_id === 'state::get') {
      const v = state.get(key(payload));
      return v !== undefined ? v : null;
    }
    if (function_id === 'state::set') {
      state.set(key(payload), payload.value);
      return { ok: true };
    }
    if (function_id === 'router::provider::register') {
      return opts.onRegister ? opts.onRegister(payload) : { ok: true, registration_token: 'tok' };
    }
    if (function_id === 'router::provider::resolve') {
      return opts.onResolve ? opts.onResolve(payload) : null;
    }
    return null;
  });
  const registerFunction = vi.fn();
  const registerTrigger = vi.fn();
  const iii = { trigger, registerFunction, registerTrigger } as unknown as ISdk;
  return { iii, trigger, registerFunction, registerTrigger, state };
}

const DECL = { id: 'testprov', display_name: 'test provider' };

describe('registerWithRouter', () => {
  beforeEach(() => {
    _resetRouterRegistrationForTests();
  });

  it('registers, persists the granted token, and unblocks resolves', async () => {
    const { iii, state } = makeIii({
      onRegister: () => ({ ok: true, id: 'testprov', registration_token: 'granted-1' }),
      onResolve: (p) => {
        expect(p).toEqual({ id: 'testprov', token: 'granted-1' });
        return {
          configured: true,
          source: 'config',
          credential: { type: 'api_key', key: 'sk' },
          api_url: null,
          max_tokens: null,
        };
      },
    });

    await registerWithRouter(iii, DECL);

    expect(state.get('llm-provider-registration/testprov')).toEqual({ token: 'granted-1' });
    const resolved = await resolveProviderViaRouter(iii, 'testprov');
    expect(resolved.configured).toBe(true);
    expect(resolved.credential).toEqual({ type: 'api_key', key: 'sk' });
  });

  it('presents the persisted token on re-registration after a restart', async () => {
    const state = new Map<string, unknown>([
      ['llm-provider-registration/testprov', { token: 'persisted-1' }],
    ]);
    const seen: unknown[] = [];
    const { iii } = makeIii({
      state,
      onRegister: (p) => {
        seen.push(p.token);
        return { ok: true, id: 'testprov', registration_token: 'persisted-1' };
      },
    });

    await registerWithRouter(iii, DECL);

    expect(seen).toEqual(['persisted-1']);
    await expect(routerRegistrationToken('testprov', 100)).resolves.toBe('persisted-1');
  });

  it('retries with backoff until the router is reachable', async () => {
    let attempts = 0;
    const { iii } = makeIii({
      onRegister: () => {
        attempts += 1;
        if (attempts < 3) throw new Error('function_not_found: router::provider::register');
        return { ok: true, id: 'testprov', registration_token: 'late-token' };
      },
    });

    await registerWithRouter(iii, DECL);

    expect(attempts).toBe(3);
    await expect(routerRegistrationToken('testprov', 100)).resolves.toBe('late-token');
  }, 10_000);

  it('a token rejection is terminal — retrying cannot fix it', async () => {
    let attempts = 0;
    const { iii } = makeIii({
      onRegister: () => {
        attempts += 1;
        throw new Error('router/registration_rejected: registration token mismatch');
      },
    });

    await registerWithRouter(iii, DECL);

    expect(attempts).toBe(1);
    await expect(routerRegistrationToken('testprov', 50)).rejects.toThrow(/no router registration/);
  });
});

describe('subscribeRouterReady', () => {
  beforeEach(() => {
    _resetRouterRegistrationForTests();
  });

  it('binds a per-provider handler to the router::ready pubsub topic', () => {
    const { iii, registerFunction, registerTrigger } = makeIii({});
    const redeclare = vi.fn();

    subscribeRouterReady(iii, 'testprov', redeclare);

    expect(registerFunction).toHaveBeenCalledWith(
      'provider::testprov::on_router_ready',
      expect.any(Function),
    );
    expect(registerTrigger).toHaveBeenCalledWith({
      type: 'subscribe',
      function_id: 'provider::testprov::on_router_ready',
      config: { topic: 'router::ready' },
    });
  });
});

describe('resolveProviderViaRouter', () => {
  beforeEach(() => {
    _resetRouterRegistrationForTests();
  });

  it('normalizes a malformed response to a none-configured result', async () => {
    _seedRouterRegistrationTokenForTests('testprov', 'tok');
    const { iii } = makeIii({ onResolve: () => 'garbage' });

    const resolved = await resolveProviderViaRouter(iii, 'testprov');
    expect(resolved).toEqual({
      configured: false,
      source: 'none',
      credential: null,
      api_url: null,
      max_tokens: null,
    });
  });

  it('fails fast with a clear error when no token landed in time', async () => {
    const { iii } = makeIii({});
    await expect(routerRegistrationToken('never-registered', 50)).rejects.toThrow(
      /llm-router down or registration still retrying/,
    );
    void iii;
  });
});
