import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { emitTurnStateChanged } from '../../src/turn-orchestrator/turn-state-write.js';

function fakeIii(): { iii: ISdk; emits: Array<{ session_id: string; event: unknown }> } {
  const emits: Array<{ session_id: string; event: unknown }> = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'stream::set') {
        const p = payload as { group_id: string; data: unknown };
        emits.push({ session_id: p.group_id, event: p.data });
        return null;
      }
      if (function_id === 'state::update') {
        return { old_value: 0 };
      }
      return null;
    }),
  } as unknown as ISdk;
  return { iii, emits };
}

describe('emitTurnStateChanged', () => {
  it('emits turn_state_changed on agent::events with group_id = session_id', async () => {
    const { iii, emits } = fakeIii();
    await emitTurnStateChanged(
      iii,
      'sess-a',
      'state:updated',
      { state: 'function_awaiting_approval', awaiting_approval: [] },
      { state: 'function_execute', awaiting_approval: null },
    );
    expect(emits).toHaveLength(1);
    expect(emits[0]?.session_id).toBe('sess-a');
    expect(emits[0]?.event).toMatchObject({
      type: 'turn_state_changed',
      event_type: 'state:updated',
      new_value: { state: 'function_awaiting_approval' },
      old_value: { state: 'function_execute' },
    });
  });

  it('swallows emit failures (logs only, never rethrows)', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('stream::set down');
      }),
    } as unknown as ISdk;
    await expect(
      emitTurnStateChanged(iii, 'sess-a', 'state:created', { state: 'provisioning' }),
    ).resolves.toBeUndefined();
  });

  it('omits old_value from the emitted event when state:created', async () => {
    const { iii, emits } = fakeIii();
    await emitTurnStateChanged(iii, 'sess-a', 'state:created', { state: 'provisioning' });
    expect(emits).toHaveLength(1);
    const event = emits[0]?.event as Record<string, unknown>;
    expect(event.type).toBe('turn_state_changed');
    expect('old_value' in event).toBe(false);
  });
});
