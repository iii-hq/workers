import { describe, expect, it, vi } from 'vitest';
import { STEP_TOPIC, handleDecisionWritten } from '../../src/approval-gate/on-decision-written.js';
import type { ISdk } from '../../src/runtime/iii.js';

describe('handleDecisionWritten', () => {
  it('extracts session_id from key and publishes turn::step_requested', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    const event = {
      event_type: 'state:updated',
      scope: 'approvals',
      key: 'sess-abc/fc-1',
      old_value: null,
      new_value: { decision: 'allow', reason: null },
      message_type: 'state',
    };

    await handleDecisionWritten(iii, event);

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('iii::durable::publish');
    expect(triggers[0]?.payload).toMatchObject({
      topic: STEP_TOPIC,
      data: { session_id: 'sess-abc' },
    });
  });

  it('handles keys with multiple slashes by taking only the prefix before the first slash', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      scope: 'approvals',
      key: 'sess-1/sub/path',
      old_value: null,
      new_value: { decision: 'deny', reason: 'test' },
      message_type: 'state',
    });

    expect(triggers[0]?.payload).toMatchObject({ data: { session_id: 'sess-1' } });
  });

  it('no-ops when the key has no slash separator', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      scope: 'approvals',
      key: 'malformed-no-slash',
      old_value: null,
      new_value: { decision: 'allow', reason: null },
      message_type: 'state',
    });
    expect(iii.trigger).not.toHaveBeenCalled();
  });

  it('no-ops when the key is missing or not a string', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleDecisionWritten(iii, {
      event_type: 'state:updated',
      scope: 'approvals',
    } as unknown);
    expect(iii.trigger).not.toHaveBeenCalled();

    await handleDecisionWritten(iii, null);
    expect(iii.trigger).not.toHaveBeenCalled();
  });

  it('handles state:created events (first decision write)', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await handleDecisionWritten(iii, {
      event_type: 'state:created',
      scope: 'approvals',
      key: 'sess-xyz/fc-1',
      old_value: null,
      new_value: { decision: 'allow', reason: null },
      message_type: 'state',
    });

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.payload).toMatchObject({ data: { session_id: 'sess-xyz' } });
  });

  it('ignores deleted events', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleDecisionWritten(iii, {
      event_type: 'state:deleted', // engine value, see iii structs.rs
      scope: 'approvals',
      key: 'sess-abc/fc-1',
      old_value: { decision: 'allow', reason: null },
      new_value: null,
      message_type: 'state',
    });
    expect(iii.trigger).not.toHaveBeenCalled();
  });
});
