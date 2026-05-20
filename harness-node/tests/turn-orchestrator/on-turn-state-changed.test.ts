import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  handleTurnStateWrite,
  isTurnStateWrite,
} from '../../src/turn-orchestrator/on-turn-state-changed.js';

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

describe('isTurnStateWrite', () => {
  it('returns true for state:created on session/<sid>/turn_state', () => {
    expect(
      isTurnStateWrite({
        event_type: 'state:created',
        key: 'session/sess-a/turn_state',
        new_value: { state: 'provisioning' },
      }),
    ).toBe(true);
  });

  it('returns true for state:updated on session/<sid>/turn_state', () => {
    expect(
      isTurnStateWrite({
        event_type: 'state:updated',
        key: 'session/sess-a/turn_state',
        new_value: { state: 'function_awaiting_approval' },
        old_value: { state: 'function_execute' },
      }),
    ).toBe(true);
  });

  it('returns false for non-turn_state agent keys', () => {
    expect(
      isTurnStateWrite({
        event_type: 'state:created',
        key: 'session/sess-a/abort_signal',
        new_value: true,
      }),
    ).toBe(false);
  });

  it('returns false for state:deleted', () => {
    expect(
      isTurnStateWrite({
        event_type: 'state:deleted',
        key: 'session/sess-a/turn_state',
      }),
    ).toBe(false);
  });
});

describe('handleTurnStateWrite', () => {
  it('emits turn_state_changed on agent::events with group_id = session_id', async () => {
    const { iii, emits } = fakeIii();
    await handleTurnStateWrite(iii, {
      event_type: 'state:updated',
      key: 'session/sess-a/turn_state',
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

  it('is a no-op when the event does not match the condition', async () => {
    const { iii, emits } = fakeIii();
    await handleTurnStateWrite(iii, {
      event_type: 'state:created',
      key: 'session/sess-a/abort_signal',
      new_value: true,
    });
    expect(emits).toEqual([]);
  });

  it('swallows emit failures (logs only, never rethrows)', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('stream::set down');
      }),
    } as unknown as ISdk;
    // Should NOT throw.
    await expect(
      handleTurnStateWrite(iii, {
        event_type: 'state:created',
        key: 'session/sess-a/turn_state',
        new_value: { state: 'provisioning' },
      }),
    ).resolves.toBeUndefined();
  });

  it('omits old_value from the emitted event when state:created', async () => {
    const { iii, emits } = fakeIii();
    await handleTurnStateWrite(iii, {
      event_type: 'state:created',
      key: 'session/sess-a/turn_state',
      new_value: { state: 'provisioning' },
    });
    expect(emits).toHaveLength(1);
    const event = emits[0]?.event as Record<string, unknown>;
    expect(event.type).toBe('turn_state_changed');
    expect('old_value' in event).toBe(false);
  });
});
