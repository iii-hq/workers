import { describe, expect, it, vi } from 'vitest';
import { TriggerAction, type ISdk } from '../../src/runtime/iii.js';
import {
  STALE_TURN_TAKEOVER_MS,
  execute,
  register,
} from '../../src/turn-orchestrator/run-start.js';
import { RunStartPayloadSchema } from '../../src/turn-orchestrator/schemas.js';
import { TURN_STEP_QUEUE } from '../../src/turn-orchestrator/state-runtime/store.js';
import { FakeSessionManager } from '../_helpers/fakeSessionManager.js';

type TriggerCall = { function_id: string; payload: unknown; action?: unknown };

function fakeIii(): { iii: ISdk; calls: TriggerCall[]; sessions: FakeSessionManager } {
  const calls: TriggerCall[] = [];
  const sessions = new FakeSessionManager();
  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      action?: unknown;
    }): Promise<R> => {
      calls.push({ function_id: req.function_id, payload: req.payload, action: req.action });
      const handled = await sessions.handle(req.function_id, req.payload);
      if (handled !== undefined) return handled as R;
      return null as R;
    },
    registerFunction: vi.fn(),
  } as unknown as ISdk;
  return { iii, calls, sessions };
}

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
    const sessions = new FakeSessionManager();
    const iii = {
      registerFunction: (fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
        registered.set(fnId, handler);
      },
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) =>
          (await sessions.handle(function_id, payload)) ?? null,
      ),
    } as unknown as ISdk;

    register(iii);
    const handler = registered.get('run::start');
    expect(handler).toBeDefined();

    const result = await handler?.(harnessRunStartPayload);
    expect(result).toEqual({ session_id: 'sess-1', started: true });
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

    await expect(handler?.({ provider: 'openai' })).rejects.toThrow();
  });
});

describe('execute', () => {
  it('saves initial session state and enqueues turn::provisioning via saveRecord wake', async () => {
    const { iii, calls } = fakeIii();

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result).toEqual({ session_id: 'sess-1', started: true });

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

  it('ensures the session exactly once, before the first append', async () => {
    const { iii, calls } = fakeIii();

    await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    const ensureCalls = calls.filter((c) => c.function_id === 'session::ensure');
    expect(ensureCalls).toHaveLength(1);
    expect(ensureCalls[0]?.payload).toEqual({ session_id: 'sess-1' });

    const ensureIdx = calls.findIndex((c) => c.function_id === 'session::ensure');
    const firstAppendIdx = calls.findIndex((c) => c.function_id === 'session::append');
    expect(ensureIdx).toBeGreaterThanOrEqual(0);
    expect(firstAppendIdx).toBeGreaterThan(ensureIdx);
  });

  function fakeIiiWithRecord(record: unknown): { iii: ISdk; calls: TriggerCall[] } {
    const calls: TriggerCall[] = [];
    const sessions = new FakeSessionManager();
    const iii = {
      trigger: async <T, R>(req: {
        function_id: string;
        payload: T;
        action?: unknown;
      }): Promise<R> => {
        calls.push({ function_id: req.function_id, payload: req.payload, action: req.action });
        if (req.function_id === 'state::get') return record as R;
        const handled = await sessions.handle(req.function_id, req.payload);
        if (handled !== undefined) return handled as R;
        return null as R;
      },
      registerFunction: vi.fn(),
    } as unknown as ISdk;
    return { iii, calls };
  }

  const inFlightRecord = (state: string, updated_at_ms = Date.now()) => ({
    session_id: 'sess-1',
    state,
    turn_count: 1,
    function_results: [],
    turn_end_emitted: false,
    started_at_ms: 1,
    updated_at_ms,
  });

  it('refuses to clobber a turn already in flight and makes no writes', async () => {
    const { iii, calls } = fakeIiiWithRecord(inFlightRecord('assistant_streaming'));

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result).toEqual({ session_id: 'sess-1', started: false, reason: 'session_busy' });
    expect(calls.some((c) => c.function_id === 'session::ensure')).toBe(false);
    expect(calls.some((c) => c.function_id === 'session::append')).toBe(false);
    expect(calls.some((c) => c.function_id === 'state::set')).toBe(false);
  });

  it('treats a parked function_awaiting_approval turn as in flight', async () => {
    const { iii } = fakeIiiWithRecord(inFlightRecord('function_awaiting_approval'));

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result.started).toBe(false);
  });

  it('starts a fresh turn when the prior turn is terminal (stopped)', async () => {
    const { iii, calls } = fakeIiiWithRecord({
      ...inFlightRecord('stopped'),
      turn_end_emitted: true,
    });

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result).toEqual({ session_id: 'sess-1', started: true });
    expect(calls.some((c) => c.function_id === 'session::append')).toBe(true);
  });

  it('takes over an in-flight record idle past the stale window (wedge recovery)', async () => {
    const stale = Date.now() - STALE_TURN_TAKEOVER_MS - 1_000;
    const { iii, calls } = fakeIiiWithRecord(inFlightRecord('finishing', stale));

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result).toEqual({ session_id: 'sess-1', started: true });
    expect(calls.some((c) => c.function_id === 'session::append')).toBe(true);
  });

  it('never stale-takes-over a parked approval (user-paced; abort is its escape)', async () => {
    const stale = Date.now() - STALE_TURN_TAKEOVER_MS - 1_000;
    const { iii } = fakeIiiWithRecord(inFlightRecord('function_awaiting_approval', stale));

    const result = await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    expect(result.started).toBe(false);
  });

  it('fails closed when the guard read fails (no clobber on a state blip)', async () => {
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        if (req.function_id === 'state::get') throw new Error('state worker unavailable');
        return null;
      }),
      registerFunction: vi.fn(),
    } as unknown as ISdk;

    await expect(execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload))).rejects.toThrow(
      /state worker unavailable/,
    );
    const calls = (iii.trigger as ReturnType<typeof vi.fn>).mock.calls as Array<
      [{ function_id: string }]
    >;
    expect(calls.some(([req]) => req.function_id === 'state::set')).toBe(false);
  });

  it('flips session status to working before the first append', async () => {
    const { iii, calls } = fakeIii();

    await execute(iii, RunStartPayloadSchema.parse(consoleRunStartPayload));

    const statusIdx = calls.findIndex((c) => c.function_id === 'session::set_status');
    const firstAppendIdx = calls.findIndex((c) => c.function_id === 'session::append');
    expect(statusIdx).toBeGreaterThanOrEqual(0);
    expect((calls[statusIdx]?.payload as { status: string }).status).toBe('working');
    expect(firstAppendIdx).toBeGreaterThan(statusIdx);
  });

  it('appends user messages with deterministic per-send entry ids and origin', async () => {
    const { iii, sessions } = fakeIii();

    await execute(iii, RunStartPayloadSchema.parse(consoleRunStartPayload));

    const appended = sessions.messageEntries('sess-1');
    expect(appended).toHaveLength(1);
    expect(appended[0]?.entry_id).toBe('msg-1-user-0');
    expect(appended[0]?.origin).toEqual({ message_id: 'msg-1' });

    await execute(iii, RunStartPayloadSchema.parse(consoleRunStartPayload));
    expect(sessions.messageEntries('sess-1')).toHaveLength(1);
  });

  it('appends without entry ids when the caller sent no message_id', async () => {
    const { iii, sessions } = fakeIii();

    await execute(iii, RunStartPayloadSchema.parse(harnessRunStartPayload));

    const appended = sessions.messageEntries('sess-1');
    expect(appended).toHaveLength(1);
    expect(appended[0]?.entry_id).toMatch(/^e-/);
  });
});
