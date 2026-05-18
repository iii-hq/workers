import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleTearingDown } from '../../src/turn-orchestrator/states/tearing-down.js';

type TriggerCall = { function_id: string; payload: unknown };

function fakeIii(handler: (call: TriggerCall) => unknown): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: { function_id: string; payload: T }): Promise<R> => {
      const call = { function_id: req.function_id, payload: req.payload };
      calls.push(call);
      const v = handler(call);
      if (v instanceof Error) throw v;
      return v as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

function runRequestGet(call: TriggerCall): boolean {
  return (
    call.function_id === 'state::get' &&
    typeof (call.payload as Record<string, unknown>)?.key === 'string' &&
    ((call.payload as Record<string, unknown>).key as string).endsWith('/run_request')
  );
}

describe('handleTearingDown with approval-aware resurrection', () => {
  it('resurrects to function_execute when consume returns entries', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const { iii } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: ['shell::exec'] };
      if (call.function_id === 'approval::consume') {
        return {
          ok: true,
          entries: [
            {
              function_call_id: 'tc-1',
              function_id: 'shell::exec',
              args: {},
              decision: 'allow',
            },
          ],
        };
      }
      return null;
    });

    await handleTearingDown(iii, rec);

    expect(rec.state).toBe('function_execute');
  });

  it('proceeds with normal teardown when consume returns no entries', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const { iii, calls } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: ['shell::exec'] };
      if (call.function_id === 'approval::consume') return { ok: true, entries: [] };
      return null;
    });

    await handleTearingDown(iii, rec);

    expect(rec.state).toBe('stopped');
    // Should NOT call sweep_session — sweep is only triggered by router::abort.
    expect(calls.some((c) => c.function_id === 'approval::sweep_session')).toBe(false);
  });

  it('calls consume even when approval_required is empty (policy is source of truth)', async () => {
    // console/web sends approval_required:[]; consume must still fire so
    // late-arriving approvals can resurrect the turn.
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const { iii, calls } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: [] };
      if (call.function_id === 'approval::consume') return { ok: true, entries: [] };
      return null;
    });

    await handleTearingDown(iii, rec);

    expect(rec.state).toBe('stopped');
    expect(calls.some((c) => c.function_id === 'approval::consume')).toBe(true);
  });

  it('proceeds with normal teardown when consume throws', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'tearing_down' };
    const { iii } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: ['shell::exec'] };
      if (call.function_id === 'approval::consume') return new Error('boom');
      return null;
    });

    await handleTearingDown(iii, rec);

    expect(rec.state).toBe('stopped');
  });
});
