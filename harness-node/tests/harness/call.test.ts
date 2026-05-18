/**
 * Contract test for `harness::call`.
 *
 * Mirrors the Rust harness's `harness::call` (see
 * `workers/harness/src/lib.rs:103-159`) by forwarding `{ function_id,
 * payload }` to `iii.trigger` and wrapping the result in an HTTP-style
 * envelope. This ensures console/web can route browser-originated chat
 * turns through a single instrumented bus function — the same pattern
 * `workers/harness/web/src/App.tsx` uses over HTTP.
 */

import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/harness/call.js';

interface RegisteredFn {
  handler: (input: unknown) => Promise<unknown>;
}

function makeFakeSdk(triggerResult: unknown = { ok: true }) {
  const registered = new Map<string, RegisteredFn>();
  const trigger = vi.fn(async (_req: unknown) => triggerResult);
  const sdk = {
    registerFunction: (fnId: string, handler: (input: unknown) => Promise<unknown>) => {
      registered.set(fnId, { handler });
      return { id: fnId, unregister: () => registered.delete(fnId) };
    },
    trigger,
  } as unknown as Parameters<typeof register>[0];
  return { sdk, registered, trigger };
}

describe('harness::call', () => {
  it('registers a handler under id "harness::call"', () => {
    const { sdk, registered } = makeFakeSdk();
    register(sdk);
    expect(registered.has('harness::call')).toBe(true);
  });

  it('forwards body.function_id and body.payload to iii.trigger', async () => {
    const { sdk, registered, trigger } = makeFakeSdk({ session_id: 'sess' });
    register(sdk);
    const handler = registered.get('harness::call')?.handler;
    if (!handler) throw new Error('handler not registered');

    const result = (await handler({
      body: {
        function_id: 'run::start',
        session_id: 'sess-1',
        message_id: 'msg-1',
        payload: { session_id: 'sess-1', provider: 'anthropic', model: 'claude-sonnet-4-6' },
      },
    })) as Record<string, unknown>;

    expect(trigger).toHaveBeenCalledTimes(1);
    const triggerArg = trigger.mock.calls[0]?.[0] as Record<string, unknown>;
    expect(triggerArg.function_id).toBe('run::start');
    expect(triggerArg.payload).toEqual({
      session_id: 'sess-1',
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
    });
    expect(result.status_code).toBe(200);
    expect(result.body).toEqual({ session_id: 'sess' });
  });

  it('falls back to top-level when body envelope is absent (WS shape)', async () => {
    const { sdk, registered, trigger } = makeFakeSdk();
    register(sdk);
    const handler = registered.get('harness::call')?.handler;
    if (!handler) throw new Error('handler not registered');

    await handler({
      function_id: 'run::start',
      session_id: 'sess-2',
      message_id: 'msg-2',
      payload: { session_id: 'sess-2', provider: 'openai', model: 'gpt-4o' },
    });

    const triggerArg = trigger.mock.calls[0]?.[0] as Record<string, unknown>;
    expect(triggerArg.function_id).toBe('run::start');
    expect(triggerArg.payload).toEqual({
      session_id: 'sess-2',
      provider: 'openai',
      model: 'gpt-4o',
    });
  });

  it('defaults payload to an empty object when omitted', async () => {
    const { sdk, registered, trigger } = makeFakeSdk();
    register(sdk);
    const handler = registered.get('harness::call')?.handler;
    if (!handler) throw new Error('handler not registered');

    await handler({ body: { function_id: 'harness::status' } });

    const triggerArg = trigger.mock.calls[0]?.[0] as Record<string, unknown>;
    expect(triggerArg.payload).toEqual({});
  });

  it('throws when function_id is missing', async () => {
    const { sdk, registered } = makeFakeSdk();
    register(sdk);
    const handler = registered.get('harness::call')?.handler;
    if (!handler) throw new Error('handler not registered');

    await expect(handler({ body: { payload: {} } })).rejects.toThrow(/missing function_id/);
  });

  it('surfaces trigger errors (no swallowing)', async () => {
    const sdk = {
      registerFunction: vi.fn((_fnId: string, handler: (input: unknown) => Promise<unknown>) => {
        triggerHandler = handler;
        return { id: 'harness::call', unregister: () => {} };
      }),
      trigger: vi.fn(async () => {
        throw new Error('boom');
      }),
    } as unknown as Parameters<typeof register>[0];
    let triggerHandler: ((input: unknown) => Promise<unknown>) | undefined;
    register(sdk);
    if (!triggerHandler) throw new Error('handler not registered');
    await expect(
      // biome-ignore lint/style/noNonNullAssertion: defined above
      triggerHandler!({ body: { function_id: 'run::start', payload: {} } }),
    ).rejects.toThrow(/boom/);
  });
});
