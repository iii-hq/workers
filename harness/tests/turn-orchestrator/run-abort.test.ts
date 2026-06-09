import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { execute, register } from '../../src/turn-orchestrator/run-abort.js';
import { newRecord, type TurnStateRecord } from '../../src/turn-orchestrator/state.js';

type TriggerCall = { function_id: string; payload: unknown };

/** iii whose state::get on turn_state returns the given persisted record. */
function fakeIiiWithRecord(record: unknown): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      calls.push({ function_id: req.function_id, payload: req.payload });
      if (req.function_id === 'state::get') return record;
      return null;
    }),
    registerFunction: vi.fn(),
  } as unknown as ISdk;
  return { iii, calls };
}

function parkedRecord(state: TurnStateRecord['state']): TurnStateRecord {
  return {
    ...newRecord('s1'),
    state,
    awaiting_approval: [
      { function_call_id: 'fc-1', function_id: 'shell::run', args: {} },
    ],
  } as TurnStateRecord;
}

function streamSets(calls: TriggerCall[]): Array<{ type?: string }> {
  return calls
    .filter((c) => c.function_id === 'stream::set')
    .map((c) => (c.payload as { data?: { type?: string } }).data ?? {});
}

describe('run::abort execute', () => {
  it('aborts a parked approval turn: aborted message, turn_end, cleared approvals, finishing', async () => {
    const { iii, calls } = fakeIiiWithRecord(parkedRecord('function_awaiting_approval'));

    const result = await execute(iii, { session_id: 's1' });

    expect(result).toEqual({ session_id: 's1', aborted: true, state: 'finishing' });

    const events = streamSets(calls).map((e) => e.type);
    expect(events).toContain('message_complete');
    expect(events).toContain('turn_end');
    // agent_end belongs to the finishing step, not the abort itself.
    expect(events).not.toContain('agent_end');

    // The record persisted to finishing with parked approvals cleared.
    const turnStateSet = calls.find(
      (c) =>
        c.function_id === 'state::set' &&
        (c.payload as { scope?: string }).scope === 'turn_state',
    );
    const saved = (turnStateSet?.payload as { value: TurnStateRecord }).value;
    expect(saved.state).toBe('finishing');
    expect(saved.awaiting_approval).toEqual([]);
    expect(saved.last_assistant?.stop_reason).toBe('aborted');
  });

  it('aborts a streaming-state record (best-effort interrupt between steps)', async () => {
    const rec = { ...newRecord('s1'), state: 'assistant_streaming' as const };
    const { iii } = fakeIiiWithRecord(rec);

    const result = await execute(iii, { session_id: 's1' });

    expect(result.aborted).toBe(true);
    expect(result.state).toBe('finishing');
  });

  it('no-ops when no turn is running', async () => {
    const { iii, calls } = fakeIiiWithRecord({ ...newRecord('s1'), state: 'stopped' });

    const result = await execute(iii, { session_id: 's1' });

    expect(result).toEqual({ session_id: 's1', aborted: false, state: 'stopped' });
    expect(calls.some((c) => c.function_id === 'state::set')).toBe(false);
  });

  it('no-ops when the session has no record', async () => {
    const { iii } = fakeIiiWithRecord(null);

    const result = await execute(iii, { session_id: 's1' });

    expect(result).toEqual({ session_id: 's1', aborted: false, state: null });
  });

  it('no-ops on a turn already finishing (it will stop on its own)', async () => {
    const { iii, calls } = fakeIiiWithRecord({ ...newRecord('s1'), state: 'finishing' });

    const result = await execute(iii, { session_id: 's1' });

    expect(result).toEqual({ session_id: 's1', aborted: false, state: 'finishing' });
    expect(calls.some((c) => c.function_id === 'state::set')).toBe(false);
  });

  it('fails closed when the record read fails', async () => {
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        if (req.function_id === 'state::get') throw new Error('state unavailable');
        return null;
      }),
      registerFunction: vi.fn(),
    } as unknown as ISdk;

    await expect(execute(iii, { session_id: 's1' })).rejects.toThrow(/state unavailable/);
  });
});

describe('register', () => {
  it('registers run::abort', () => {
    const registered = new Map<string, unknown>();
    const iii = {
      registerFunction: (id: string, handler: unknown) => registered.set(id, handler),
    } as unknown as ISdk;

    register(iii);

    expect(registered.has('run::abort')).toBe(true);
  });
});
