import { describe, expect, it } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import {
  approvalRequiredEnabled,
  handleAwaiting,
} from '../../src/turn-orchestrator/states/assistant.js';

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

describe('approvalRequiredEnabled', () => {
  it('returns true for a non-empty approval_required array', () => {
    expect(approvalRequiredEnabled({ approval_required: ['shell::exec'] })).toBe(true);
  });
  it('returns false for an empty array', () => {
    expect(approvalRequiredEnabled({ approval_required: [] })).toBe(false);
  });
  it('returns false when the field is missing or non-array', () => {
    expect(approvalRequiredEnabled({})).toBe(false);
    expect(approvalRequiredEnabled({ approval_required: 'shell::exec' })).toBe(false);
    expect(approvalRequiredEnabled(null)).toBe(false);
  });
});

describe('handleAwaiting with approval consume', () => {
  it('transitions to function_execute when consume returns entries', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'awaiting_assistant' };
    const { iii, calls } = fakeIii((call) => {
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

    await handleAwaiting(iii, rec);

    expect(rec.state).toBe('function_execute');
    // Turn count must NOT increment on resurrection — the resolved approvals
    // are completing the same turn that paused.
    expect(rec.turn_count).toBe(0);
    // Should have saved prepared calls.
    const setCalls = calls.filter(
      (c) =>
        c.function_id === 'state::set' &&
        typeof (c.payload as Record<string, unknown>)?.key === 'string' &&
        ((c.payload as Record<string, unknown>).key as string).endsWith('/function_schemas') ===
          false,
    );
    expect(setCalls.length).toBeGreaterThan(0);
  });

  it('falls through to existing flow when consume returns no entries', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'awaiting_assistant' };
    const { iii } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: ['shell::exec'] };
      if (call.function_id === 'approval::consume') return { ok: true, entries: [] };
      return null;
    });

    await handleAwaiting(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.turn_count).toBe(1);
  });

  it('calls consume even when approval_required is empty (policy is source of truth)', async () => {
    // console/web intentionally sends approval_required:[] — gating on the
    // list would skip the consume-on-resume branch. handleAwaiting now
    // always calls consume; if it returns empty entries, normal flow.
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'awaiting_assistant' };
    const { iii, calls } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: [] };
      if (call.function_id === 'approval::consume') return { ok: true, entries: [] };
      return null;
    });

    await handleAwaiting(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(calls.some((c) => c.function_id === 'approval::consume')).toBe(true);
  });

  it('continues to the existing flow when consume throws', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'awaiting_assistant' };
    const { iii } = fakeIii((call) => {
      if (runRequestGet(call)) return { approval_required: ['shell::exec'] };
      if (call.function_id === 'approval::consume') return new Error('consume_boom');
      return null;
    });

    await handleAwaiting(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
  });
});
