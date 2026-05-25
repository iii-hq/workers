import { describe, expect, it } from 'vitest';
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
