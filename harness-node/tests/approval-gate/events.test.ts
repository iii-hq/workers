import { describe, expect, it } from 'vitest';
import {
  emitApprovalRequested,
  emitApprovalResolved,
  emitApprovalWakeFailed,
} from '../../src/approval-gate/events.js';
import type { ISdk } from '../../src/runtime/iii.js';

type TriggerCall = { function_id: string; payload: unknown };

function fakeIii(): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: { function_id: string; payload: T }): Promise<R> => {
      calls.push({ function_id: req.function_id, payload: req.payload });
      return undefined as unknown as R;
    },
  } as unknown as ISdk;
  return { iii, calls };
}

describe('approval-gate events', () => {
  it('emitApprovalRequested writes the expected envelope to agent::events', async () => {
    const { iii, calls } = fakeIii();
    await emitApprovalRequested(iii, 's1', {
      function_call_id: 'tc-1',
      function_id: 'shell::exec',
      args: { command: 'date' },
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]?.function_id).toBe('stream::set');
    const payload = calls[0]?.payload as Record<string, unknown>;
    expect(payload.stream_name).toBe('agent::events');
    expect(payload.group_id).toBe('s1');
    const data = payload.data as Record<string, unknown>;
    expect(data.type).toBe('approval_requested');
    expect(data.function_call_id).toBe('tc-1');
    expect(data.tool_call_id).toBe('tc-1');
    expect(data.function_id).toBe('shell::exec');
    expect(data.tool_name).toBe('shell::exec');
    expect(data).not.toHaveProperty('expires_at');
  });

  it('emitApprovalResolved writes the expected envelope', async () => {
    const { iii, calls } = fakeIii();
    await emitApprovalResolved(iii, 's1', {
      function_call_id: 'tc-1',
      decision: 'allow',
      reason: null,
    });
    const data = (calls[0]?.payload as Record<string, unknown>).data as Record<string, unknown>;
    expect(data.type).toBe('approval_resolved');
    expect(data.function_call_id).toBe('tc-1');
    expect(data.tool_call_id).toBe('tc-1');
    expect(data.decision).toBe('allow');
    expect(data.reason).toBeNull();
  });

  it('emitApprovalWakeFailed writes the expected envelope with error string', async () => {
    const { iii, calls } = fakeIii();
    await emitApprovalWakeFailed(iii, 's1', 'resume timeout');
    const data = (calls[0]?.payload as Record<string, unknown>).data as Record<string, unknown>;
    expect(data.type).toBe('approval_wake_failed');
    expect(data.error).toBe('resume timeout');
  });
});
