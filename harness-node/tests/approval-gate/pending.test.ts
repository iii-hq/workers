import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  handleListPending,
  handleResolve,
  handleResolveWithEvents,
  resumeSession,
} from '../../src/approval-gate/pending.js';
import { InMemoryStateBus } from '../../src/approval-gate/state-bus.js';
import { buildPendingRecord, pendingKey } from '../../src/approval-gate/types.js';
import type { ISdk } from '../../src/runtime/iii.js';

type TriggerCall = { function_id: string; payload: unknown; timeoutMs?: number };

function fakeIii(handler: (call: TriggerCall) => unknown): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      timeoutMs?: number;
    }): Promise<R> => {
      const call = {
        function_id: req.function_id,
        payload: req.payload,
        timeoutMs: req.timeoutMs,
      };
      calls.push(call);
      const result = handler(call);
      if (result instanceof Error) throw result;
      return result as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

describe('handleResolve', () => {
  it('marks status resolved with decision and resolved_at on success', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('s1', 'tc-1', 'write', {}, Date.now(), 60_000),
    );
    const out = (await handleResolve(bus, 'approvals', {
      function_call_id: 'tc-1',
      session_id: 's1',
      decision: 'allow',
    })) as Record<string, unknown>;
    expect(out.ok).toBe(true);
    const stored = (await bus.get('approvals', pendingKey('s1', 'tc-1'))) as Record<
      string,
      unknown
    >;
    expect(stored.status).toBe('resolved');
    expect(stored.decision).toBe('allow');
    expect(typeof stored.resolved_at).toBe('number');
  });

  it('rejects already-resolved entries', async () => {
    const bus = new InMemoryStateBus();
    const rec = buildPendingRecord('s1', 'tc-1', 'write', {}, 0, 60_000);
    rec.status = 'resolved';
    rec.decision = 'allow';
    await bus.set('approvals', pendingKey('s1', 'tc-1'), rec);
    const out = (await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'tc-1',
      decision: 'deny',
    })) as Record<string, unknown>;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('already_resolved');
  });

  it('force-denies on expired pending and returns timed_out', async () => {
    const bus = new InMemoryStateBus();
    const expired = buildPendingRecord('s1', 'tc-1', 'write', {}, 1_000, 60_000);
    expired.expires_at = 500; // way in the past relative to Date.now()
    await bus.set('approvals', pendingKey('s1', 'tc-1'), expired);
    const out = (await handleResolve(bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'tc-1',
      decision: 'allow',
    })) as Record<string, unknown>;
    expect(out.ok).toBe(false);
    expect(out.error).toBe('timed_out');
    const stored = (await bus.get('approvals', pendingKey('s1', 'tc-1'))) as Record<
      string,
      unknown
    >;
    expect(stored.status).toBe('resolved');
    expect(stored.decision).toBe('deny');
    expect(stored.reason).toBe('timed_out');
  });

  it('errors on missing id or bad decision', async () => {
    const bus = new InMemoryStateBus();
    expect(((await handleResolve(bus, 'approvals', {})) as Record<string, unknown>).error).toBe(
      'missing_id',
    );
    expect(
      (
        (await handleResolve(bus, 'approvals', {
          session_id: 's1',
          function_call_id: 'tc-1',
          decision: 'weird',
        })) as Record<string, unknown>
      ).error,
    ).toBe('bad_decision');
  });
});

describe('handleListPending', () => {
  it('returns only pending entries for the session', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('s1', 'tc-1', 'write', {}, 0, 60_000),
    );
    const resolved = buildPendingRecord('s1', 'tc-2', 'write', {}, 0, 60_000);
    resolved.status = 'resolved';
    resolved.decision = 'allow';
    await bus.set('approvals', pendingKey('s1', 'tc-2'), resolved);
    await bus.set(
      'approvals',
      pendingKey('other', 'tc-3'),
      buildPendingRecord('other', 'tc-3', 'write', {}, 0, 60_000),
    );
    const out = (await handleListPending(bus, 'approvals', { session_id: 's1' })) as {
      pending: Array<Record<string, unknown>>;
    };
    expect(out.pending).toHaveLength(1);
    expect(out.pending[0]?.function_call_id).toBe('tc-1');
  });
});

describe('resumeSession', () => {
  it('fires a single iii::durable::publish on turn::step_requested (no polling)', async () => {
    const { iii, calls } = fakeIii(() => null);
    await expect(resumeSession(iii, 's1')).resolves.toBeUndefined();
    expect(calls).toHaveLength(1);
    expect(calls[0]?.function_id).toBe('iii::durable::publish');
    expect(calls[0]?.payload).toEqual({
      topic: 'turn::step_requested',
      data: { session_id: 's1' },
    });
    // No explicit timeoutMs — let iii's default handle backstop.
    expect(calls[0]?.timeoutMs).toBeUndefined();
  });

  it('propagates publish errors so the wrapper can emit approval_wake_failed', async () => {
    const { iii } = fakeIii(() => new Error('bus closed'));
    await expect(resumeSession(iii, 's1')).rejects.toThrow(/bus closed/);
  });
});

describe('handleResolveWithEvents', () => {
  it('on resolve success emits approval_resolved and publishes turn::step_requested', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('s1', 'tc-1', 'write', {}, Date.now(), 60_000),
    );
    const { iii, calls } = fakeIii(() => undefined);

    const out = (await handleResolveWithEvents(iii, bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'tc-1',
      decision: 'allow',
    })) as Record<string, unknown>;

    expect(out.ok).toBe(true);
    const streamCalls = calls.filter((c) => c.function_id === 'stream::set');
    expect(streamCalls).toHaveLength(1);
    const data = (streamCalls[0]?.payload as Record<string, unknown>).data as Record<
      string,
      unknown
    >;
    expect(data.type).toBe('approval_resolved');
    expect(data.decision).toBe('allow');
    const publishCalls = calls.filter((c) => c.function_id === 'iii::durable::publish');
    expect(publishCalls).toHaveLength(1);
    expect(publishCalls[0]?.payload).toEqual({
      topic: 'turn::step_requested',
      data: { session_id: 's1' },
    });
  });

  it('emits approval_wake_failed when the resume publish throws', async () => {
    const bus = new InMemoryStateBus();
    await bus.set(
      'approvals',
      pendingKey('s1', 'tc-1'),
      buildPendingRecord('s1', 'tc-1', 'write', {}, Date.now(), 60_000),
    );
    const { iii, calls } = fakeIii((call) => {
      if (call.function_id === 'iii::durable::publish') return new Error('wake boom');
      return undefined;
    });

    const out = (await handleResolveWithEvents(iii, bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'tc-1',
      decision: 'allow',
    })) as Record<string, unknown>;

    expect(out.ok).toBe(true);
    const wakeFailedCall = calls
      .filter((c) => c.function_id === 'stream::set')
      .map((c) => (c.payload as Record<string, unknown>).data as Record<string, unknown>)
      .find((d) => d.type === 'approval_wake_failed');
    expect(wakeFailedCall).toBeDefined();
    expect(wakeFailedCall?.error).toMatch(/wake boom/);
  });

  it('does not emit or resume when handleResolve returns ok:false', async () => {
    const bus = new InMemoryStateBus();
    const { iii, calls } = fakeIii(() => ({ resumed: true }));

    const out = (await handleResolveWithEvents(iii, bus, 'approvals', {
      session_id: 's1',
      function_call_id: 'tc-1',
      decision: 'allow',
    })) as Record<string, unknown>;

    expect(out.ok).toBe(false);
    expect(calls).toHaveLength(0);
  });
});
