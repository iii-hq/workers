import { describe, expect, it, vi } from 'vitest';
import { TriggerAction, type ISdk } from '../../src/runtime/iii.js';
import {
  execute,
  handleApprovalDecisionWrite,
  isApprovalDecisionWrite,
  parseApprovalDecisionWrite,
} from '../../src/turn-orchestrator/on-approval.js';
import { ApprovalDecisionEventSchema } from '../../src/turn-orchestrator/schemas.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

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

  it('accepts deny and aborted decisions', () => {
    expect(
      ApprovalDecisionEventSchema.parse({
        ...matchingEvent,
        new_value: { decision: 'deny', reason: 'policy' },
      }),
    ).toEqual({ session_id: 'sess-abc' });
    expect(
      ApprovalDecisionEventSchema.parse({
        ...matchingEvent,
        new_value: { decision: 'aborted', reason: 'x' },
      }),
    ).toEqual({ session_id: 'sess-abc' });
  });

  it('rejects values without a decision', () => {
    expect(() =>
      ApprovalDecisionEventSchema.parse({ ...matchingEvent, new_value: { reason: 'x' } }),
    ).toThrow();
  });

  it('rejects keys that are not <sid>/<cid> shaped', () => {
    expect(() =>
      ApprovalDecisionEventSchema.parse({ ...matchingEvent, key: 'session/sess-abc/turn_state' }),
    ).toThrow();
    expect(() =>
      ApprovalDecisionEventSchema.parse({ ...matchingEvent, key: 'no-slash' }),
    ).toThrow();
  });

  it('rejects state:deleted and nested wrappers', () => {
    expect(() =>
      ApprovalDecisionEventSchema.parse({ ...matchingEvent, event_type: 'state:deleted' }),
    ).toThrow();
    expect(() => ApprovalDecisionEventSchema.parse({ payload: matchingEvent })).toThrow();
    expect(() => ApprovalDecisionEventSchema.parse(null)).toThrow();
  });
});

function mockIiiWithTurnState(rec: ReturnType<typeof newRecord>): {
  iii: ISdk;
  triggers: Array<{ function_id: string; payload: unknown; action?: unknown }>;
} {
  const triggers: Array<{ function_id: string; payload: unknown; action?: unknown }> = [];
  const iii = {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown; action?: unknown }) => {
      if (req.function_id === 'state::get') return rec;
      triggers.push(req);
      return null;
    }),
  } as unknown as ISdk;
  return { iii, triggers };
}

describe('parseApprovalDecisionWrite condition', () => {
  it('matches an approvals decision write and extracts session_id', () => {
    expect(parseApprovalDecisionWrite(matchingEvent)).toEqual({ session_id: 'sess-abc' });
    expect(isApprovalDecisionWrite(matchingEvent)).toBe(true);
  });

  it('skips writes with no decision and non-<sid>/<cid> keys', () => {
    expect(parseApprovalDecisionWrite({ ...matchingEvent, new_value: { reason: 'x' } })).toBeNull();
    expect(
      parseApprovalDecisionWrite({ ...matchingEvent, key: 'session/s/turn_state' }),
    ).toBeNull();
  });
});

describe('handleApprovalDecisionWrite', () => {
  it('extracts session_id and enqueues turn::{state}', async () => {
    const rec = newRecord('sess-abc');
    rec.state = 'function_awaiting_approval';
    const { iii, triggers } = mockIiiWithTurnState(rec);

    await handleApprovalDecisionWrite(iii, matchingEvent);

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('turn::function_awaiting_approval');
    expect(triggers[0]?.payload).toEqual({ session_id: 'sess-abc' });
    expect(triggers[0]?.action).toEqual(TriggerAction.Enqueue({ queue: 'turn-step' }));
  });

  it('no-ops on a non-matching event', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleApprovalDecisionWrite(iii, { ...matchingEvent, new_value: { reason: 'x' } });
    expect(iii.trigger).not.toHaveBeenCalled();
  });
});

describe('on-approval execute', () => {
  it('enqueues turn::{state} on the turn-step queue', async () => {
    const rec = newRecord('sess-abc');
    rec.state = 'function_awaiting_approval';
    const { iii, triggers } = mockIiiWithTurnState(rec);

    await execute(iii, { session_id: 'sess-abc' });

    expect(triggers[0]?.function_id).toBe('turn::function_awaiting_approval');
    expect(triggers[0]?.action).toEqual(TriggerAction.Enqueue({ queue: 'turn-step' }));
  });
});
