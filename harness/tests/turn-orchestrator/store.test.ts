import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  createTurnStore,
  shouldWakeStep,
} from '../../src/turn-orchestrator/state-runtime/store.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import { FakeSessionManager } from '../_helpers/fakeSessionManager.js';

function fakeIii(): { iii: ISdk; emits: Array<{ session_id: string; event: unknown }> } {
  const emits: Array<{ session_id: string; event: unknown }> = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'stream::set') {
        const p = payload as { group_id: string; data: unknown };
        emits.push({ session_id: p.group_id, event: p.data });
        return null;
      }
      if (function_id === 'state::set') {
        return { old_value: null, new_value: (payload as { value: unknown }).value };
      }
      if (function_id === 'state::update') {
        return { old_value: 0 };
      }
      return null;
    }),
  } as unknown as ISdk;
  return { iii, emits };
}

describe('saveRecord turn_state_changed emission', () => {
  it('emits turn_state_changed on agent::events with group_id = session_id', async () => {
    const { iii, emits } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');
    rec.state = 'function_awaiting_approval';
    const previous = { ...rec, state: 'function_execute' as const };

    await store.saveRecord(rec, previous);

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
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');
    await expect(store.saveRecord(rec)).resolves.toBeUndefined();
  });

  it('omits old_value from the emitted event when state:created', async () => {
    const { iii, emits } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');
    rec.state = 'provisioning';

    await store.saveRecord(rec);

    expect(emits).toHaveLength(1);
    const event = emits[0]?.event as Record<string, unknown>;
    expect(event.type).toBe('turn_state_changed');
    expect(event.event_type).toBe('state:created');
    expect('old_value' in event).toBe(false);
  });
});

describe('saveRecord no-op suppression', () => {
  it('does not emit turn_state_changed when the view is unchanged (timestamp-only delta)', async () => {
    const { iii, emits } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');
    const previous = { ...rec, updated_at_ms: rec.updated_at_ms + 1000 };

    await store.saveRecord(rec, previous);

    expect(emits).toHaveLength(0);
  });

  it('does not emit on an idempotent re-save (previous === current)', async () => {
    const { iii, emits } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');

    await store.saveRecord(rec, rec);

    expect(emits).toHaveLength(0);
  });

  it('still emits when a view field other than state changes', async () => {
    const { iii, emits } = fakeIii();
    const store = createTurnStore(iii);
    const rec = newRecord('sess-a');
    const previous = { ...rec, turn_count: rec.turn_count + 1 };

    await store.saveRecord(rec, previous);

    expect(emits).toHaveLength(1);
  });
});

describe('session-manager call reduction', () => {
  function recordingIii(): { iii: ISdk; calls: string[] } {
    const calls: string[] = [];
    const sessions = new FakeSessionManager();
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
          calls.push(function_id);
          const handled = await sessions.handle(function_id, payload);
          if (handled !== undefined) return handled;
          return null;
        },
      ),
    } as unknown as ISdk;
    return { iii, calls };
  }

  it('loadMessages reads the window in one paginated read without re-ensuring', async () => {
    const { iii, calls } = recordingIii();
    await createTurnStore(iii).loadMessages('sess-a');
    expect(calls.filter((c) => c === 'session::messages')).toHaveLength(1);
    expect(calls).not.toContain('session::ensure');
  });

  it('appendMessages writes without re-ensuring the session', async () => {
    const { iii, calls } = recordingIii();
    const msg = {
      role: 'user' as const,
      content: [{ type: 'text' as const, text: 'hi' }],
      timestamp: 1,
    };
    await createTurnStore(iii).appendMessages('sess-a', [msg]);
    expect(calls.filter((c) => c === 'session::append')).toHaveLength(1);
    expect(calls).not.toContain('session::ensure');
  });

  it('appendMessages issues one session::append per message', async () => {
    const { iii, calls } = recordingIii();
    const mk = (text: string) => ({
      role: 'user' as const,
      content: [{ type: 'text' as const, text }],
      timestamp: 1,
    });
    await createTurnStore(iii).appendMessages('sess-a', [mk('a'), mk('b'), mk('c')]);
    expect(calls.filter((c) => c === 'session::append')).toHaveLength(3);
  });

  it('appendMessages on an empty list triggers nothing', async () => {
    const { iii, calls } = recordingIii();
    await createTurnStore(iii).appendMessages('sess-a', []);
    expect(calls).not.toContain('session::append');
  });

  it('ensureSession is the sole trigger of session::ensure', async () => {
    const { iii, calls } = recordingIii();
    await createTurnStore(iii).ensureSession('sess-a');
    expect(calls.filter((c) => c === 'session::ensure')).toHaveLength(1);
  });
});

describe('shouldWakeStep', () => {
  it('accepts first write to a stepable state', () => {
    expect(shouldWakeStep(null, 'provisioning')).toBe(true);
  });

  it('accepts transitions to another stepable state', () => {
    expect(shouldWakeStep('provisioning', 'assistant_streaming')).toBe(true);
    expect(shouldWakeStep('assistant_streaming', 'function_execute')).toBe(true);
  });

  it('rejects terminal state (stopped)', () => {
    expect(shouldWakeStep('function_execute', 'stopped')).toBe(false);
  });

  it('rejects function_awaiting_approval (orchestrator parks here)', () => {
    expect(shouldWakeStep('function_execute', 'function_awaiting_approval')).toBe(false);
  });

  it('rejects same-state writes', () => {
    expect(shouldWakeStep('function_execute', 'function_execute')).toBe(false);
  });
});
