import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  handleStepableRecordWrite,
  parseStepableWrite,
} from '../../src/turn-orchestrator/on-record-written.js';

describe('parseStepableWrite condition', () => {
  it('accepts minimal required fields (matches TurnStateWriteEventSchema callers)', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:created',
        key: 'session/sess-a/turn_state',
        new_value: { state: 'provisioning' },
      }),
    ).toEqual({ session_id: 'sess-a' });
  });

  it('matches turn_state writes with a non-terminal, non-awaiting state', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:created',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { state: 'provisioning' },
        message_type: 'state',
      }),
    ).toEqual({ session_id: 'sess-abc' });

    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'provisioning' },
        new_value: { state: 'awaiting_assistant' },
        message_type: 'state',
      }),
    ).toEqual({ session_id: 'sess-abc' });
  });

  it('rejects terminal state (stopped)', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'tearing_down' },
        new_value: { state: 'stopped' },
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('rejects function_awaiting_approval (orchestrator parks here)', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'function_prepare' },
        new_value: { state: 'function_awaiting_approval' },
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('rejects state:deleted', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:deleted',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'provisioning' },
        new_value: null,
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('rejects non-turn_state keys in the agent scope', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: null,
        new_value: true,
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('rejects same-state writes (old_value.state === new_value.state)', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'function_prepare' },
        new_value: { state: 'function_prepare' },
        message_type: 'state',
      }),
    ).toBeNull();

    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: { state: 'function_prepare' },
        new_value: { state: 'function_execute' },
        message_type: 'state',
      }),
    ).toEqual({ session_id: 'sess-abc' });
  });

  it('rejects publish envelope shapes — state triggers receive flat events', () => {
    expect(
      parseStepableWrite({
        topic: 'turn::step_requested',
        data: { session_id: 'sess-abc' },
      }),
    ).toBeNull();
  });

  it('rejects nested payload wrappers (no in-repo caller uses them)', () => {
    const inner = {
      event_type: 'state:created',
      key: 'session/sess-abc/turn_state',
      new_value: { state: 'provisioning' },
    };
    expect(parseStepableWrite({ data: inner })).toBeNull();
    expect(parseStepableWrite({ payload: inner })).toBeNull();
  });

  it('rejects null, undefined, and non-object events', () => {
    expect(parseStepableWrite(null)).toBeNull();
    expect(parseStepableWrite(undefined)).toBeNull();
    expect(parseStepableWrite('state:created')).toBeNull();
  });

  it('rejects writes whose new_value lacks a string state', () => {
    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { not_state: 'provisioning' },
        message_type: 'state',
      }),
    ).toBeNull();

    expect(
      parseStepableWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: null,
        message_type: 'state',
      }),
    ).toBeNull();
  });
});

describe('handleStepableRecordWrite', () => {
  it('extracts session_id and publishes turn::step_requested', async () => {
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
    expect(triggers[0]?.function_id).toBe('iii::durable::publish');
    expect(triggers[0]?.payload).toEqual({
      topic: 'turn::step_requested',
      data: { session_id: 'sess-abc' },
    });
  });

  it('no-ops when the event is not stepable (direct invoke bypasses engine condition)', async () => {
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

  it('no-ops on same-state turn_state updates (direct invoke bypasses engine condition)', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleStepableRecordWrite(iii, {
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: { state: 'function_prepare' },
      new_value: { state: 'function_prepare' },
      message_type: 'state',
    });
    expect(iii.trigger).not.toHaveBeenCalled();
  });
});
