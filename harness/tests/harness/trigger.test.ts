/**
 * Contract test for `harness::trigger`.
 *
 * console/web forwards chat kickoff via a single flat payload over WS.
 */

import { describe, expect, it, vi } from 'vitest';
import { register } from '../../src/harness/trigger.js';

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

const runStartPayload = {
  session_id: 'sess-1',
  provider: 'anthropic',
  model: 'claude-sonnet-4-6',
  messages: [
    {
      role: 'user' as const,
      content: [{ type: 'text' as const, text: 'hi' }],
      timestamp: Date.now(),
    },
  ],
};

describe('harness::trigger', () => {
  it('registers a handler under id "harness::trigger"', () => {
    const { sdk, registered } = makeFakeSdk();
    register(sdk);
    expect(registered.has('harness::trigger')).toBe(true);
  });

  it('forwards payload to run::start', async () => {
    const { sdk, registered, trigger } = makeFakeSdk({ session_id: 'sess', started: true });
    register(sdk);
    const handler = registered.get('harness::trigger')?.handler;
    if (!handler) throw new Error('handler not registered');

    const result = (await handler({
      session_id: 'sess-1',
      message_id: 'msg-1',
      payload: runStartPayload,
    })) as Record<string, unknown>;

    expect(trigger).toHaveBeenCalledTimes(1);
    const triggerArg = trigger.mock.calls[0]?.[0] as Record<string, unknown>;
    expect(triggerArg.function_id).toBe('run::start');
    expect(triggerArg.payload).toMatchObject(runStartPayload);
    expect(triggerArg.payload).toMatchObject({
      system_prompt: '',
    });
    expect(result.status_code).toBe(200);
    expect(result.body).toEqual({ session_id: 'sess', started: true });
  });

  it('returns 409 when run::start reports the session is busy', async () => {
    const { sdk, registered } = makeFakeSdk({
      session_id: 'sess',
      started: false,
      reason: 'session_busy',
    });
    register(sdk);
    const handler = registered.get('harness::trigger')?.handler;
    if (!handler) throw new Error('handler not registered');

    const result = (await handler({
      session_id: 'sess-1',
      payload: runStartPayload,
    })) as Record<string, unknown>;

    expect(result.status_code).toBe(409);
    expect(result.body).toEqual({ session_id: 'sess', started: false, reason: 'session_busy' });
  });

  it('rejects invalid run::start payload', async () => {
    const { sdk, registered } = makeFakeSdk();
    register(sdk);
    const handler = registered.get('harness::trigger')?.handler;
    if (!handler) throw new Error('handler not registered');

    await expect(
      handler({
        session_id: 'sess-1',
        payload: { provider: 'openai' },
      }),
    ).rejects.toThrow();
  });

  it('surfaces trigger errors (no swallowing)', async () => {
    const sdk = {
      registerFunction: vi.fn((_fnId: string, handler: (input: unknown) => Promise<unknown>) => {
        triggerHandler = handler;
        return { id: 'harness::trigger', unregister: () => {} };
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
      triggerHandler!({
        payload: runStartPayload,
      }),
    ).rejects.toThrow(/boom/);
  });
});
