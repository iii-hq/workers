import { describe, expect, it, vi } from 'vitest';
import { TriggerAction } from '../../src/runtime/iii.js';
import type { ISdk } from '../../src/runtime/iii.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import { shouldWakeStep, wakeFromRecord, wakeState } from '../../src/turn-orchestrator/wake.js';

describe('shouldWakeStep', () => {
  it('accepts first write to a stepable state', () => {
    expect(shouldWakeStep(null, 'provisioning')).toBe(true);
  });

  it('accepts transitions to another stepable state', () => {
    expect(shouldWakeStep('provisioning', 'assistant_streaming')).toBe(true);
    expect(shouldWakeStep('assistant_streaming', 'function_execute')).toBe(true);
  });

  it('rejects terminal state (stopped)', () => {
    expect(shouldWakeStep('tearing_down', 'stopped')).toBe(false);
  });

  it('rejects function_awaiting_approval (orchestrator parks here)', () => {
    expect(shouldWakeStep('function_execute', 'function_awaiting_approval')).toBe(false);
  });

  it('rejects same-state writes', () => {
    expect(shouldWakeStep('function_execute', 'function_execute')).toBe(false);
  });
});

describe('wakeState', () => {
  it('enqueues turn::{state} on the turn-step FIFO queue', async () => {
    const triggers: Array<{ function_id: string; payload: unknown; action?: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown; action?: unknown }) => {
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await wakeState(iii, 'sess-abc', 'assistant_streaming');

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

    await expect(wakeState(iii, 'sess-abc', 'provisioning')).resolves.toBeUndefined();
  });
});

describe('wakeFromRecord', () => {
  it('enqueues turn::{currentState} from persisted record', async () => {
    const rec = newRecord('sess-x');
    rec.state = 'function_awaiting_approval';
    const triggers: Array<{ function_id: string; payload: unknown; action?: unknown }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string; payload: unknown; action?: unknown }) => {
        if (req.function_id === 'state::get') {
          return rec;
        }
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await wakeFromRecord(iii, 'sess-x');

    expect(triggers).toHaveLength(1);
    expect(triggers[0]?.function_id).toBe('turn::function_awaiting_approval');
    expect(triggers[0]?.payload).toEqual({ session_id: 'sess-x' });
  });

  it('no-ops when session is stopped', async () => {
    const rec = newRecord('sess-y');
    rec.state = 'stopped';
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        if (req.function_id === 'state::get') return rec;
        return null;
      }),
    } as unknown as ISdk;

    await wakeFromRecord(iii, 'sess-y');
    expect(iii.trigger).toHaveBeenCalledTimes(1);
  });

  it('no-ops when session is failed (no turn::failed handler exists)', async () => {
    const rec = newRecord('sess-z');
    rec.state = 'failed';
    const triggers: Array<{ function_id: string }> = [];
    const iii = {
      trigger: vi.fn(async (req: { function_id: string }) => {
        if (req.function_id === 'state::get') return rec;
        triggers.push(req);
        return null;
      }),
    } as unknown as ISdk;

    await wakeFromRecord(iii, 'sess-z');
    // only the state::get read — no enqueue of an unregistered turn::failed
    expect(iii.trigger).toHaveBeenCalledTimes(1);
    expect(triggers).toHaveLength(0);
  });
});
