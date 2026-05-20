import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  handleStepableRecordWrite,
  isStepableRecordWrite,
} from '../../src/turn-orchestrator/on-record-written.js';
import { STEP_FN_ID } from '../../src/turn-orchestrator/subscriber.js';

describe('isStepableRecordWrite condition', () => {
  it('matches turn_state writes with a non-terminal, non-awaiting state', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:created',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { state: 'provisioning' },
        message_type: 'state',
      }),
    ).toBe(true);

    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'provisioning' },
        new_value: { state: 'awaiting_assistant' },
        message_type: 'state',
      }),
    ).toBe(true);
  });

  it('rejects terminal state (stopped)', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'tearing_down' },
        new_value: { state: 'stopped' },
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('rejects function_awaiting_approval (orchestrator parks here)', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'function_prepare' },
        new_value: { state: 'function_awaiting_approval' },
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('rejects state:deleted', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:deleted',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'provisioning' },
        new_value: null,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('rejects non-turn_state keys in the agent scope', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: null,
        new_value: true,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('rejects same-state writes (old_value.state === new_value.state)', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'function_prepare' },
        new_value: { state: 'function_prepare' },
        message_type: 'state',
      }),
    ).toBe(false);

    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'function_prepare' },
        new_value: { state: 'function_execute' },
        message_type: 'state',
      }),
    ).toBe(true);
  });

  it('rejects writes whose new_value lacks a string state', () => {
    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { not_state: 'provisioning' },
        message_type: 'state',
      }),
    ).toBe(false);

    expect(
      isStepableRecordWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: null,
        message_type: 'state',
      }),
    ).toBe(false);
  });
});

describe('handleStepableRecordWrite', () => {
  it('extracts session_id and invokes turn::step directly', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await handleStepableRecordWrite(iii, {
      event_type: 'state:created',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: null,
      new_value: { state: 'provisioning' },
      message_type: 'state',
    });

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe(STEP_FN_ID);
    expect(triggers[0]?.payload).toEqual({ session_id: 'sess-abc' });
  });

  it('no-ops when key does not match the turn_state pattern', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleStepableRecordWrite(iii, {
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/abort_signal',
      old_value: null,
      new_value: true,
      message_type: 'state',
    });
    expect(iii.trigger).not.toHaveBeenCalled();
  });

  it('falls back to durable publish when the direct turn::step invoke fails', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        // Fail the direct turn::step invoke; let the durable publish succeed.
        if (req.function_id === STEP_FN_ID) {
          throw new Error('engine down');
        }
        return null;
      }),
    } as unknown as ISdk;

    await handleStepableRecordWrite(iii, {
      event_type: 'state:created',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: null,
      new_value: { state: 'provisioning' },
      message_type: 'state',
    });

    expect(triggers).toHaveLength(2);
    expect(triggers[0]?.function_id).toBe(STEP_FN_ID);
    expect(triggers[1]?.function_id).toBe('iii::durable::publish');
    expect(triggers[1]?.payload).toEqual({
      topic: 'turn::step_requested',
      data: { session_id: 'sess-abc' },
    });
  });

  it('swallows when BOTH the direct invoke and durable publish fallback fail', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('engine down');
      }),
    } as unknown as ISdk;

    await expect(
      handleStepableRecordWrite(iii, {
        event_type: 'state:created',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { state: 'provisioning' },
        message_type: 'state',
      }),
    ).resolves.toBeUndefined();
  });
});
