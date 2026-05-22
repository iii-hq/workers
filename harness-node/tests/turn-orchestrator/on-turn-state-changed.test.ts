import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  handleTurnStateWrite,
  isTurnStateWrite,
  parseTurnStateWrite,
  TurnStateWriteEventSchema,
} from '../../src/turn-orchestrator/on-turn-state-changed.js';

const canonicalCreated = {
  event_type: 'state:created' as const,
  scope: 'agent' as const,
  key: 'session/sess-a/turn_state',
  old_value: null,
  new_value: { state: 'provisioning' },
  message_type: 'state' as const,
};

const canonicalUpdated = {
  event_type: 'state:updated' as const,
  scope: 'agent' as const,
  key: 'session/sess-a/turn_state',
  old_value: { state: 'function_execute' },
  new_value: { state: 'function_awaiting_approval' },
  message_type: 'state' as const,
};

function fakeIii(): { iii: ISdk; emits: Array<{ session_id: string; event: unknown }> } {
  const emits: Array<{ session_id: string; event: unknown }> = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'stream::set') {
        const p = payload as { group_id: string; data: unknown };
        emits.push({ session_id: p.group_id, event: p.data });
        return null;
      }
      return null;
    }),
  } as unknown as ISdk;
  return { iii, emits };
}

describe('TurnStateWriteEventSchema / isTurnStateWrite', () => {
  it('accepts the canonical agent state write shape from the iii engine', () => {
    expect(TurnStateWriteEventSchema.parse(canonicalCreated)).toEqual({
      session_id: 'sess-a',
      event_type: 'state:created',
      new_value: { state: 'provisioning' },
    });

    expect(TurnStateWriteEventSchema.parse(canonicalUpdated)).toEqual({
      session_id: 'sess-a',
      event_type: 'state:updated',
      new_value: { state: 'function_awaiting_approval' },
      old_value: { state: 'function_execute' },
    });

    expect(isTurnStateWrite(canonicalCreated)).toBe(true);
    expect(isTurnStateWrite(canonicalUpdated)).toBe(true);
  });

  it('accepts minimal shapes without optional engine metadata', () => {
    expect(
      parseTurnStateWrite({
        event_type: 'state:created',
        key: 'session/sess-a/turn_state',
        new_value: { state: 'provisioning' },
      }),
    ).toEqual({
      session_id: 'sess-a',
      event_type: 'state:created',
      new_value: { state: 'provisioning' },
    });
  });

  it('rejects nested payload wrappers (no in-repo caller uses them)', () => {
    expect(() => TurnStateWriteEventSchema.parse({ payload: canonicalCreated })).toThrow();
    expect(() => TurnStateWriteEventSchema.parse({ data: canonicalCreated })).toThrow();
    expect(isTurnStateWrite({ payload: canonicalCreated })).toBe(false);
  });

  it('rejects non-turn_state agent keys', () => {
    expect(
      isTurnStateWrite({
        ...canonicalCreated,
        key: 'session/sess-a/abort_signal',
        new_value: { state: 'true' },
      }),
    ).toBe(false);
  });

  it('rejects state:deleted', () => {
    expect(
      isTurnStateWrite({
        event_type: 'state:deleted',
        scope: 'agent',
        key: 'session/sess-a/turn_state',
        old_value: { state: 'provisioning' },
        new_value: null,
        message_type: 'state',
      }),
    ).toBe(false);
  });

  it('rejects missing key, empty session id segment, or malformed new_value', () => {
    expect(isTurnStateWrite({ ...canonicalCreated, key: undefined })).toBe(false);
    expect(isTurnStateWrite({ ...canonicalCreated, key: 'session//turn_state' })).toBe(false);
    expect(isTurnStateWrite({ ...canonicalCreated, new_value: { not_state: 'x' } })).toBe(false);
    expect(isTurnStateWrite({ ...canonicalCreated, new_value: null })).toBe(false);
    expect(isTurnStateWrite(null)).toBe(false);
    expect(isTurnStateWrite(undefined)).toBe(false);
  });
});

describe('handleTurnStateWrite', () => {
  it('emits turn_state_changed on agent::events with group_id = session_id', async () => {
    const { iii, emits } = fakeIii();
    await handleTurnStateWrite(iii, {
      ...canonicalUpdated,
      new_value: { state: 'function_awaiting_approval', awaiting_approval: [] },
      old_value: { state: 'function_execute', awaiting_approval: null },
    });
    expect(emits).toHaveLength(1);
    expect(emits[0]?.session_id).toBe('sess-a');
    expect(emits[0]?.event).toMatchObject({
      type: 'turn_state_changed',
      event_type: 'state:updated',
      new_value: { state: 'function_awaiting_approval' },
      old_value: { state: 'function_execute' },
    });
  });

  it('no-ops when the event does not match the condition (direct invoke bypasses engine condition)', async () => {
    const { iii, emits } = fakeIii();
    await handleTurnStateWrite(iii, {
      event_type: 'state:created',
      scope: 'agent',
      key: 'session/sess-a/abort_signal',
      old_value: null,
      new_value: true,
      message_type: 'state',
    });
    expect(emits).toEqual([]);
  });

  it('swallows emit failures (logs only, never rethrows)', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('stream::set down');
      }),
    } as unknown as ISdk;
    await expect(handleTurnStateWrite(iii, canonicalCreated)).resolves.toBeUndefined();
  });

  it('omits old_value from the emitted event when state:created', async () => {
    const { iii, emits } = fakeIii();
    await handleTurnStateWrite(iii, canonicalCreated);
    expect(emits).toHaveLength(1);
    const event = emits[0]?.event as Record<string, unknown>;
    expect(event.type).toBe('turn_state_changed');
    expect('old_value' in event).toBe(false);
  });
});
