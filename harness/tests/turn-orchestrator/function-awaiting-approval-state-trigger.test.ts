import { describe, expect, it, vi } from 'vitest';
import { TriggerAction, type ISdk } from '../../src/runtime/iii.js';
import { handleApprovalStateWrite } from '../../src/turn-orchestrator/function-awaiting-approval/process.js';
import { ApprovalDecisionEventSchema } from '../../src/turn-orchestrator/schemas.js';

const matchingEvent = {
  event_type: 'state:created' as const,
  scope: 'approvals' as const,
  key: 'sess-abc/fc-1',
  old_value: null,
  new_value: { decision: 'allow', reason: null },
  message_type: 'state',
};

describe('ApprovalDecisionEventSchema', () => {
  it('extracts session_id from the <sid>/<cid> key', () => {
    expect(ApprovalDecisionEventSchema.parse(matchingEvent)).toEqual({ session_id: 'sess-abc' });
  });
});

describe('handleApprovalStateWrite', () => {
  it('enqueues turn::function_awaiting_approval on a decision write', async () => {
    const triggers: Array<{ function_id: string; payload: unknown; action?: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown; action?: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await handleApprovalStateWrite(iii, matchingEvent);

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('turn::function_awaiting_approval');
    expect(triggers[0]?.payload).toEqual({ session_id: 'sess-abc' });
    expect(triggers[0]?.action).toEqual(TriggerAction.Enqueue({ queue: 'turn-step' }));
  });

  it('no-ops on a non-matching event', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleApprovalStateWrite(iii, { ...matchingEvent, new_value: { reason: 'x' } });
    expect(iii.trigger).not.toHaveBeenCalled();
  });
});
