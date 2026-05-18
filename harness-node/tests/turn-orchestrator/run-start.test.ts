import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  RESUME_FUNCTION_ID,
  buildResumePlan,
  buildResumeRecord,
  executeResume,
} from '../../src/turn-orchestrator/run-start.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';

type TriggerCall = { function_id: string; payload: unknown };

function fakeIii(handler: (call: TriggerCall) => unknown): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: { function_id: string; payload: T }): Promise<R> => {
      const call = { function_id: req.function_id, payload: req.payload };
      calls.push(call);
      return handler(call) as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

function turnStateGet(call: TriggerCall): boolean {
  return (
    call.function_id === 'state::get' &&
    typeof (call.payload as Record<string, unknown>)?.key === 'string' &&
    ((call.payload as Record<string, unknown>).key as string).endsWith('/turn_state')
  );
}

describe('RESUME_FUNCTION_ID', () => {
  it('matches the Rust run::resume function id', () => {
    expect(RESUME_FUNCTION_ID).toBe('run::resume');
  });
});

describe('buildResumeRecord', () => {
  it('returns a fresh non-terminal record that preserves turn_count and max_turns', () => {
    const stopped: TurnStateRecord = {
      ...newRecord('sess-1', 4),
      state: 'stopped',
      turn_count: 3,
      last_assistant: null,
      pending_function_calls: [{ id: 'tc-1', function_id: 'shell::exec', arguments: {} }],
    };

    const resumed = buildResumeRecord(stopped);

    expect(resumed.session_id).toBe('sess-1');
    expect(resumed.state).toBe('provisioning');
    expect(resumed.turn_count).toBe(3);
    expect(resumed.max_turns).toBe(4);
    expect(resumed.last_assistant).toBeNull();
    expect(resumed.pending_function_calls).toEqual([]);
    expect(resumed.function_results).toEqual([]);
  });
});

describe('buildResumePlan', () => {
  it('returns a resume record only for terminal records', () => {
    const stopped: TurnStateRecord = { ...newRecord('s1'), state: 'stopped' };
    const active: TurnStateRecord = { ...newRecord('s1'), state: 'function_execute' };
    expect(buildResumePlan(stopped)).not.toBeNull();
    expect(buildResumePlan(active)).toBeNull();
  });
});

describe('executeResume', () => {
  it('rejects missing session_id', async () => {
    const { iii } = fakeIii(() => null);
    await expect(executeResume(iii, {})).rejects.toThrow(/session_id/);
  });

  it('errors when the session record is not found', async () => {
    const { iii } = fakeIii((call) => (turnStateGet(call) ? null : null));
    await expect(executeResume(iii, { session_id: 'unknown' })).rejects.toThrow(
      /unknown session: unknown/,
    );
  });

  it('returns resumed:true and publishes a step when the record is terminal', async () => {
    const terminal: TurnStateRecord = {
      ...newRecord('s1', 5),
      state: 'stopped',
      turn_count: 2,
    };
    const { iii, calls } = fakeIii((call) => {
      if (turnStateGet(call)) return terminal;
      return null;
    });
    const out = (await executeResume(iii, { session_id: 's1' })) as Record<string, unknown>;
    expect(out.ok).toBe(true);
    expect(out.session_id).toBe('s1');
    expect(out.resumed).toBe(true);

    const setCalls = calls.filter(
      (c) =>
        c.function_id === 'state::set' &&
        typeof (c.payload as Record<string, unknown>)?.key === 'string' &&
        ((c.payload as Record<string, unknown>).key as string).endsWith('/turn_state'),
    );
    expect(setCalls.length).toBeGreaterThanOrEqual(1);
    const publishCalls = calls.filter((c) => c.function_id === 'iii::durable::publish');
    expect(publishCalls).toHaveLength(1);
  });

  it('returns resumed:false and writes nothing when the record is active', async () => {
    const active: TurnStateRecord = { ...newRecord('s1'), state: 'function_execute' };
    const { iii, calls } = fakeIii((call) => {
      if (turnStateGet(call)) return active;
      return null;
    });
    const out = (await executeResume(iii, { session_id: 's1' })) as Record<string, unknown>;
    expect(out.ok).toBe(true);
    expect(out.resumed).toBe(false);

    const setCalls = calls.filter(
      (c) =>
        c.function_id === 'state::set' &&
        typeof (c.payload as Record<string, unknown>)?.key === 'string' &&
        ((c.payload as Record<string, unknown>).key as string).endsWith('/turn_state'),
    );
    expect(setCalls).toHaveLength(0);
    const publishCalls = calls.filter((c) => c.function_id === 'iii::durable::publish');
    expect(publishCalls).toHaveLength(0);
  });
});
