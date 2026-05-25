import { describe, expect, it, vi } from 'vitest';
import { TriggerAction } from '../../src/runtime/iii.js';
import type { ISdk } from '../../src/runtime/iii.js';
import {
  createTurnStore,
  parseFlatMessages,
  shouldWakeStep,
} from '../../src/turn-orchestrator/state-runtime/store.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';

describe('parseFlatMessages', () => {
  it('returns the array when messages are objects', () => {
    const messages = [{ role: 'user', content: [], timestamp: 1 }];
    expect(parseFlatMessages(messages)).toEqual(messages);
  });

  it('returns [] for null, undefined, and non-arrays', () => {
    expect(parseFlatMessages(null)).toEqual([]);
    expect(parseFlatMessages(undefined)).toEqual([]);
    expect(parseFlatMessages('bad')).toEqual([]);
    expect(parseFlatMessages({})).toEqual([]);
  });
});

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

describe('shouldWakeStep', () => {
  it('accepts first write to a stepable state', () => {
    expect(shouldWakeStep(null, 'provisioning')).toBe(true);
  });

  it('accepts transitions to another stepable state', () => {
    expect(shouldWakeStep('provisioning', 'assistant_streaming')).toBe(true);
    expect(shouldWakeStep('assistant_streaming', 'function_execute')).toBe(true);
  });

  it('rejects terminal state (stopped)', () => {
    expect(shouldWakeStep('steering_check', 'stopped')).toBe(false);
  });

  it('rejects function_awaiting_approval (orchestrator parks here)', () => {
    expect(shouldWakeStep('function_execute', 'function_awaiting_approval')).toBe(false);
  });

  it('rejects same-state writes', () => {
    expect(shouldWakeStep('function_execute', 'function_execute')).toBe(false);
  });
});

describe('TurnStore.wakeStep', () => {
  it('enqueues turn::{state} on the turn-step FIFO queue', async () => {
    const triggers: Array<{ function_id: string; payload: unknown; action?: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown; action?: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await createTurnStore(iii).wakeStep('sess-abc', 'assistant_streaming');

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('turn::assistant_streaming');
    expect(triggers[0]?.payload).toEqual({ session_id: 'sess-abc' });
    expect(triggers[0]?.action).toEqual(TriggerAction.Enqueue({ queue: 'turn-step' }));
  });

  it('swallows enqueue failures (logs only, never rethrows)', async () => {
    const iii = {
      trigger: vi.fn(async () => {
        throw new Error('queue down');
      }),
    } as unknown as ISdk;

    await expect(createTurnStore(iii).wakeStep('sess-abc', 'provisioning')).resolves.toBeUndefined();
  });
});

describe('TurnStore.wakeFromRecord', () => {
  it('enqueues turn::{currentState} from persisted record', async () => {
    const rec = newRecord('sess-x');
    rec.state = 'function_awaiting_approval';
    const triggers: Array<{ function_id: string; payload: unknown; action?: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown; action?: unknown }) => {
        if (req.function_id === 'state::get') return rec;
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await createTurnStore(iii).wakeFromRecord('sess-x');

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('turn::function_awaiting_approval');
    expect(triggers[0]?.payload).toEqual({ session_id: 'sess-x' });
  });

  it('no-ops when session is stopped', async () => {
    const rec = newRecord('sess-y');
    rec.state = 'stopped';
    const turnTriggers: string[] = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        if (req.function_id === 'state::get') return rec;
        if (req.function_id.startsWith('turn::')) turnTriggers.push(req.function_id);
        return null;
      }),
    } as unknown as ISdk;

    await createTurnStore(iii).wakeFromRecord('sess-y');
    expect(turnTriggers).toHaveLength(0);
  });

  it('no-ops when session is failed (no turn::failed handler exists)', async () => {
    const rec = newRecord('sess-z');
    rec.state = 'failed';
    const turnTriggers: string[] = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        if (req.function_id === 'state::get') return rec;
        if (req.function_id.startsWith('turn::')) turnTriggers.push(req.function_id);
        return null;
      }),
    } as unknown as ISdk;

    await createTurnStore(iii).wakeFromRecord('sess-z');
    expect(turnTriggers).toHaveLength(0);
  });
});
