import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  STEP_TOPIC,
  handleAbortSignalWrite,
  isAbortSignalWrite,
} from '../../src/turn-orchestrator/on-abort-signal.js';

describe('isAbortSignalWrite condition', () => {
  it('matches session/<id>/abort_signal with new_value === true', () => {
    expect(
      isAbortSignalWrite({
        event_type: 'state:created',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: null,
        new_value: true,
        message_type: 'state',
      }),
    ).toBe(true);
  });

  it('matches state:updated transitioning to true', () => {
    expect(
      isAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: false,
        new_value: true,
        message_type: 'state',
      }),
    ).toBe(true);
  });

  it('skips state:deleted', () => {
    expect(
      isAbortSignalWrite({
        event_type: 'state:deleted',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: true,
        new_value: null,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('skips writes that set the signal to false (idempotent clears)', () => {
    expect(
      isAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: true,
        new_value: false,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('skips non-abort_signal keys in the agent scope', () => {
    expect(
      isAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { state: 'function_execute' },
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('skips top-level non-session keys', () => {
    expect(
      isAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'harness/index/abc/last_session_id',
        old_value: null,
        new_value: 'sess-1',
        message_type: 'state',
      }),
    ).toBe(false);
  });
});

describe('handleAbortSignalWrite', () => {
  it('extracts session_id and publishes turn::step_requested', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await handleAbortSignalWrite(iii, {
      event_type: 'state:created',
      scope: 'agent',
      key: 'session/sess-abc/abort_signal',
      old_value: null,
      new_value: true,
      message_type: 'state',
    });

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('iii::durable::publish');
    expect(triggers[0]?.payload).toMatchObject({
      topic: STEP_TOPIC,
      data: { session_id: 'sess-abc' },
    });
  });

  it('no-ops when key does not match the abort_signal pattern', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleAbortSignalWrite(iii, {
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/turn_state',
      old_value: null,
      new_value: {},
      message_type: 'state',
    });
    expect(iii.trigger).not.toHaveBeenCalled();
  });
});
