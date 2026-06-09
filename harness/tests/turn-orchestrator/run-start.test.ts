import { describe, expect, it, vi } from 'vitest';
import { TriggerAction, type ISdk } from '../../src/runtime/iii.js';
import { execute, register } from '../../src/turn-orchestrator/run-start.js';
import { RunStartPayloadSchema } from '../../src/turn-orchestrator/schemas.js';
import { TURN_STEP_QUEUE } from '../../src/turn-orchestrator/state-runtime/store.js';

type TriggerCall = { function_id: string; payload: unknown; action?: unknown };

function fakeIii(): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      action?: unknown;
    }): Promise<R> => {
      calls.push({ function_id: req.function_id, payload: req.payload, action: req.action });
      return null as R;
    },
    registerFunction: vi.fn(),
  } as unknown as ISdk;
  return { iii, calls };
}

/** Shape console/web sends inside harness::trigger payload (real.ts). */
const consoleRunStartPayload = {
  session_id: 'sess-1',
  message_id: 'msg-1',
  provider: 'anthropic',
  model: 'claude-sonnet-4-6',
  mode: 'agent' as const,
  messages: [
    {
      role: 'user' as const,
      content: [{ type: 'text' as const, text: 'hi' }],
      timestamp: Date.now(),
    },
  ],
};

/** Minimal shape harness/trigger.test.ts forwards to run::start. */
const harnessRunStartPayload = {
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

describe('RunStartPayloadSchema', () => {
  it('accepts the console/web payload shape', () => {
    expect(RunStartPayloadSchema.parse(consoleRunStartPayload)).toMatchObject({
      session_id: 'sess-1',
      message_id: 'msg-1',
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      mode: 'agent',
      system_prompt: '',
      messages: consoleRunStartPayload.messages,
    });
  });

  it('accepts the minimal harness::trigger test payload with defaults', () => {
    expect(RunStartPayloadSchema.parse(harnessRunStartPayload)).toMatchObject({
      session_id: 'sess-1',
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      system_prompt: '',
      messages: harnessRunStartPayload.messages,
    });
  });

  it('rejects harness::trigger envelope shapes — run::start receives payload only', () => {
    expect(() =>
      RunStartPayloadSchema.parse({
        session_id: 'outer',
        message_id: 'msg-1',
        payload: harnessRunStartPayload,
      }),
    ).toThrow();
  });

  it('rejects nested payload/data wrappers (no in-repo caller uses them)', () => {
    expect(() => RunStartPayloadSchema.parse({ data: harnessRunStartPayload })).toThrow();
    expect(() => RunStartPayloadSchema.parse({ payload: harnessRunStartPayload })).toThrow();
  });

  it('rejects missing or invalid required fields', () => {
    expect(() => RunStartPayloadSchema.parse({})).toThrow();
    expect(() => RunStartPayloadSchema.parse({ session_id: '' })).toThrow();
    expect(() => RunStartPayloadSchema.parse({ session_id: 's1' })).toThrow();
    expect(() => RunStartPayloadSchema.parse({ session_id: 's1', provider: 'p' })).toThrow();
    expect(() =>
      RunStartPayloadSchema.parse({ session_id: 42, provider: 'p', model: 'm' }),
    ).toThrow();
    expect(() =>
      RunStartPayloadSchema.parse({ session_id: 's1', provider: 'p', model: 'm', mode: 'invalid' }),
    ).toThrow();
    expect(() => RunStartPayloadSchema.parse(null)).toThrow();
    expect(() => RunStartPayloadSchema.parse(undefined)).toThrow();
  });
});

describe('register', () => {
  it('registers run::start and parses payload at the unknown boundary', async () => {
    const registered = new Map<string, (payload: unknown) => Promise<unknown>>();
    const iii = {
      registerFunction: (fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
        registered.set(fnId, handler);
      },
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;

    register(iii);
    const handler = registered.get('run::start');
    expect(handler).toBeDefined();

    const result = await handler!(harnessRunStartPayload);
    expect(result).toEqual({ session_id: 'sess-1' });
  });

  it('rejects invalid payloads at register boundary', async () => {
    const registered = new Map<string, (payload: unknown) => Promise<unknown>>();
    const iii = {
      registerFunction: (fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
        registered.set(fnId, handler);
      },
      trigger: vi.fn(async () => null),
    } as unknown as ISdk;

    register(iii);
    const handler = registered.get('run::start');
    expect(handler).toBeDefined();

    await expect(handler!({ provider: 'openai' })).rejects.toThrow();
  });
});

describe('execute', () => {
  it('saves initial session state and enqueues turn::provisioning via saveRecord wake', async () => {
    const { iii, calls } = fakeIii();

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result).toEqual({ session_id: 'sess-1' });

    const turnStateSet = calls.find(
      (c) =>
        c.function_id === 'state::set' &&
        (c.payload as { scope?: string; key?: string }).scope === 'turn_state' &&
        (c.payload as { scope?: string; key?: string }).key === 'sess-1',
    );
    expect(turnStateSet).toBeDefined();
    expect((turnStateSet?.payload as { value: { state: string } }).value.state).toBe(
      'provisioning',
    );

    const wake = calls.find((c) => c.function_id === 'turn::provisioning');
    expect(wake).toBeDefined();
    expect(wake?.payload).toEqual({ session_id: 'sess-1' });
    expect(wake?.action).toEqual(TriggerAction.Enqueue({ queue: TURN_STEP_QUEUE }));
  });

  it('ensures the session tree exactly once, before the first append', async () => {
    const { iii, calls } = fakeIii();

    await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    // Single ensure per run — later loadMessages/appendMessages no longer re-ensure.
    const ensureCalls = calls.filter((c) => c.function_id === 'session-tree::ensure');
    expect(ensureCalls).toHaveLength(1);
    expect(ensureCalls[0]?.payload).toEqual({ session_id: 'sess-1' });

    // The single ensure must precede the run's first tree write (append batch).
    const ensureIdx = calls.findIndex((c) => c.function_id === 'session-tree::ensure');
    const firstAppendIdx = calls.findIndex((c) => c.function_id === 'session-tree::append_batch');
    expect(ensureIdx).toBeGreaterThanOrEqual(0);
    expect(firstAppendIdx).toBeGreaterThan(ensureIdx);
  });
});
