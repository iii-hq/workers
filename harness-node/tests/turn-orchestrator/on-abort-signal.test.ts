import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  AbortSignalWriteEventSchema,
  execute,
  handleAbortSignalWrite,
  isAbortSignalWrite,
  parseAbortSignalWrite,
} from '../../src/turn-orchestrator/on-abort-signal.js';

const matchingEvent = {
  event_type: 'state:created' as const,
  scope: 'agent' as const,
  key: 'session/sess-abc/abort_signal',
  old_value: null,
  new_value: true as const,
  message_type: 'state',
};

describe('AbortSignalWriteEventSchema', () => {
  it('accepts the agent state write shape from state::set / engine triggers', () => {
    expect(AbortSignalWriteEventSchema.parse(matchingEvent)).toEqual({
      session_id: 'sess-abc',
    });
  });

  it('rejects durable publish envelope shapes (not a state trigger event)', () => {
    expect(() =>
      AbortSignalWriteEventSchema.parse({
        topic: 'turn::step_requested',
        data: { session_id: 's1' },
      }),
    ).toThrow();
  });

  it('rejects nested payload wrappers', () => {
    expect(() => AbortSignalWriteEventSchema.parse({ payload: matchingEvent })).toThrow();
    expect(() => AbortSignalWriteEventSchema.parse({ data: matchingEvent })).toThrow();
  });

  it('rejects missing key, wrong new_value, or non-abort_signal keys', () => {
    expect(() => AbortSignalWriteEventSchema.parse({})).toThrow();
    expect(() =>
      AbortSignalWriteEventSchema.parse({
        ...matchingEvent,
        key: 'session/sess-abc/turn_state',
      }),
    ).toThrow();
    expect(() =>
      AbortSignalWriteEventSchema.parse({
        ...matchingEvent,
        new_value: false,
      }),
    ).toThrow();
    expect(() =>
      AbortSignalWriteEventSchema.parse({
        ...matchingEvent,
        event_type: 'state:deleted',
      }),
    ).toThrow();
    expect(() => AbortSignalWriteEventSchema.parse(null)).toThrow();
  });
});

describe('parseAbortSignalWrite condition', () => {
  it('matches session/<id>/abort_signal with new_value === true', () => {
    expect(parseAbortSignalWrite(matchingEvent)).toEqual({ session_id: 'sess-abc' });
    expect(isAbortSignalWrite(matchingEvent)).toBe(true);
  });

  it('matches state:updated transitioning to true', () => {
    const event = {
      event_type: 'state:updated' as const,
      scope: 'agent' as const,
      key: 'session/sess-abc/abort_signal',
      old_value: false,
      new_value: true as const,
      message_type: 'state',
    };
    expect(parseAbortSignalWrite(event)).toEqual({ session_id: 'sess-abc' });
  });

  it('skips state:deleted', () => {
    expect(
      parseAbortSignalWrite({
        event_type: 'state:deleted',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: true,
        new_value: null,
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('skips writes that set the signal to false (idempotent clears)', () => {
    expect(
      parseAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/abort_signal',
        old_value: true,
        new_value: false,
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('skips non-abort_signal keys in the agent scope', () => {
    expect(
      parseAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'session/sess-abc/turn_state',
        old_value: null,
        new_value: { state: 'function_execute' },
        message_type: 'state',
      }),
    ).toBeNull();
  });

  it('skips top-level non-session keys', () => {
    expect(
      parseAbortSignalWrite({
        event_type: 'state:updated',
        scope: 'agent',
        key: 'harness/index/abc/last_session_id',
        old_value: null,
        new_value: 'sess-1',
        message_type: 'state',
      }),
    ).toBeNull();
  });
});

describe('execute', () => {
  it('publishes turn::step_requested via durable publish', async () => {
    const triggers: Array<{ function_id: string; payload: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await execute(iii, { session_id: 'sess-abc' });

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('iii::durable::publish');
    expect(triggers[0]?.payload).toEqual({
      topic: 'turn::step_requested',
      data: { session_id: 'sess-abc' },
    });
  });

  it('swallows publish failures (logs only, never rethrows)', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('durable down');
      }),
    } as unknown as ISdk;

    await expect(execute(iii, { session_id: 'sess-abc' })).resolves.toBeUndefined();
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

    await handleAbortSignalWrite(iii, matchingEvent);

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('iii::durable::publish');
    expect(triggers[0]?.payload).toEqual({
      topic: 'turn::step_requested',
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

  it('no-ops when new_value is not true (direct invoke bypasses engine condition)', async () => {
    const iii = { trigger: vi.fn() } as unknown as ISdk;
    await handleAbortSignalWrite(iii, {
      event_type: 'state:updated',
      scope: 'agent',
      key: 'session/sess-abc/abort_signal',
      old_value: true,
      new_value: false,
      message_type: 'state',
    });
    expect(iii.trigger).not.toHaveBeenCalled();
  });
});
