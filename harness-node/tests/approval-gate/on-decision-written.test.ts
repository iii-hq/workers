import { describe, expect, it, vi } from 'vitest';
import {
  CONDITION_FN_ID,
  STEP_FN_ID,
  TRIGGER_FN_ID,
  handleDecisionWritten,
  isDecisionWrite,
} from '../../src/approval-gate/on-decision-written.js';
import { fakeIii } from './_helpers/fakeIii.js';

describe('handleDecisionWritten', () => {
  it('extracts session_id from key and triggers turn::step', async () => {
    const { iii, stepCalls } = fakeIii();
    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      scope: 'approvals',
      key: 'sess-abc/fc-1',
      old_value: null,
      new_value: { decision: 'allow', reason: null },
      message_type: 'state',
    });
    expect(stepCalls).toHaveLength(1);
    expect(stepCalls[0]).toEqual({ session_id: 'sess-abc' });
  });

  it('preserves slashes in the function_call_id half (tolerant parse)', async () => {
    const { iii, stepCalls } = fakeIii();
    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      key: 'sess-1/sub/path',
      new_value: { decision: 'deny', reason: 'test' },
    });
    expect(stepCalls[0]).toEqual({ session_id: 'sess-1' });
  });

  it('handles state:created events (first decision write)', async () => {
    const { iii, stepCalls } = fakeIii();
    await handleDecisionWritten(iii, {
      event_type: 'state:created',
      key: 'sess-xyz/fc-1',
      new_value: { decision: 'allow', reason: null },
    });
    expect(stepCalls).toEqual([{ session_id: 'sess-xyz' }]);
  });

  it('no-ops when the key has no slash separator', async () => {
    const { iii, calls } = fakeIii();
    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      key: 'malformed-no-slash',
      new_value: { decision: 'allow' },
    });
    expect(calls).toHaveLength(0);
  });

  it('no-ops when the event is missing required fields (no key)', async () => {
    const { iii, calls } = fakeIii();
    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      scope: 'approvals',
    });
    expect(calls).toHaveLength(0);
  });

  it('no-ops on null / non-object input', async () => {
    const { iii, calls } = fakeIii();
    await handleDecisionWritten(iii, null);
    await handleDecisionWritten(iii, 'string-payload');
    expect(calls).toHaveLength(0);
  });

  it('still triggers when handed state:deleted directly (handler is not the filter)', async () => {
    // The engine-side condition function filters state:deleted before this
    // handler is invoked. This test asserts the handler doesn't have defensive
    // double-filtering that would mask a regression in registration wiring.
    const { iii, stepCalls } = fakeIii();
    await handleDecisionWritten(iii, {
      event_type: 'state:deleted',
      key: 'sess-x/fc-1',
      new_value: null,
    });
    expect(stepCalls).toHaveLength(1);
  });

  // R4 — silent-warn semantics regression guard. Event-trigger handlers MUST
  // NOT throw; a thrown handler crashes trigger dispatch for everyone.
  it('does not throw when iii.trigger fails (R4 silent-warn guard)', async () => {
    const { iii } = fakeIii();
    (iii.trigger as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('downstream boom'));
    await expect(
      handleDecisionWritten(iii, {
        event_type: 'state:updated',
        key: 'sess-1/fc-1',
        new_value: { decision: 'allow' },
      }),
    ).resolves.toBeUndefined();
  });
});

describe('isDecisionWrite condition', () => {
  it('returns true for state:created with a decision in new_value', () => {
    expect(
      isDecisionWrite({
        event_type: 'state:created',
        key: 'sess/fc',
        new_value: { decision: 'allow', reason: null },
      }),
    ).toBe(true);
  });

  it('returns true for state:updated with a decision in new_value', () => {
    expect(
      isDecisionWrite({
        event_type: 'state:updated',
        key: 'sess/fc',
        new_value: { decision: 'deny', reason: 'user' },
      }),
    ).toBe(true);
  });

  it('returns false for state:deleted', () => {
    expect(
      isDecisionWrite({
        event_type: 'state:deleted',
        key: 'sess/fc',
        new_value: null,
      }),
    ).toBe(false);
  });

  it('returns false when new_value has no decision field', () => {
    expect(
      isDecisionWrite({
        event_type: 'state:updated',
        key: 'sess/fc',
        new_value: { somethingElse: true },
      }),
    ).toBe(false);
  });

  it('returns false when new_value is missing or null', () => {
    expect(
      isDecisionWrite({
        event_type: 'state:created',
        key: 'sess/fc',
        new_value: null,
      }),
    ).toBe(false);
    expect(isDecisionWrite({ event_type: 'state:created', key: 'sess/fc' })).toBe(false);
  });

  it('returns false for non-object / unknown event_type', () => {
    expect(isDecisionWrite(null)).toBe(false);
    expect(isDecisionWrite('whatever')).toBe(false);
    expect(
      isDecisionWrite({ event_type: 'state:weird', key: 'x', new_value: { decision: 'allow' } }),
    ).toBe(false);
  });
});

describe('exported function ids', () => {
  it('stable wire ids', () => {
    expect(TRIGGER_FN_ID).toBe('approval::on_decision_written');
    expect(CONDITION_FN_ID).toBe('approval::is_decision_write');
    expect(STEP_FN_ID).toBeTypeOf('string');
  });
});
