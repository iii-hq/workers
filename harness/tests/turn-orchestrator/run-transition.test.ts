import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { runTransition } from '../../src/turn-orchestrator/run-transition.js';
import {
  type TurnStateRecord,
  newRecord,
  transitionTo,
} from '../../src/turn-orchestrator/state.js';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('runTransition', () => {
  it('throws when the session record is missing, without running the handler', async () => {
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(null);
    const handle = vi.fn();

    await expect(
      runTransition({} as ISdk, 'provisioning', handle, { session_id: 'missing' }),
    ).rejects.toThrow('turn::provisioning invariant: missing session missing');
    expect(handle).not.toHaveBeenCalled();
  });

  it('returns a stale skip without running the handler or saving', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    const saveRecord = vi.spyOn(persistence, 'saveRecord').mockResolvedValue();
    const handle = vi.fn();

    const result = await runTransition({} as ISdk, 'provisioning', handle, { session_id: 's1' });

    expect(result).toEqual({ ok: true, skipped: true, reason: 'stale' });
    expect(handle).not.toHaveBeenCalled();
    expect(saveRecord).not.toHaveBeenCalled();
  });

  it('runs the handler and threads the pre-mutation snapshot into saveRecord', async () => {
    const iii = {} as ISdk;
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    const saveRecord = vi.spyOn(persistence, 'saveRecord').mockResolvedValue();
    const handle = vi.fn(async (_iii: ISdk, r: TurnStateRecord) => {
      transitionTo(r, 'assistant_streaming');
    });

    const result = await runTransition(iii, 'provisioning', handle, { session_id: 's1' });

    expect(handle).toHaveBeenCalledWith(iii, rec);
    expect(saveRecord).toHaveBeenCalledWith(
      iii,
      rec,
      expect.objectContaining({ state: 'provisioning' }),
    );
    expect(result).toEqual({
      ok: true,
      from_state: 'provisioning',
      to_state: 'assistant_streaming',
    });
  });

  it('snapshots a deep copy so in-place handler mutation does not leak into previous', async () => {
    const iii = {} as ISdk;
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'function_execute' };
    rec.awaiting_approval = [];
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    let captured: TurnStateRecord | null | undefined;
    vi.spyOn(persistence, 'saveRecord').mockImplementation(async (_i, _r, previous) => {
      captured = previous;
    });
    const handle = vi.fn(async (_iii: ISdk, r: TurnStateRecord) => {
      r.awaiting_approval?.push({ function_call_id: 'fc-1', function_id: 'f', args: {} });
      transitionTo(r, 'function_awaiting_approval');
    });

    await runTransition(iii, 'function_execute', handle, { session_id: 's1' });

    // The snapshot reflects state BEFORE the handler ran, even though the
    // handler mutated rec.awaiting_approval in place.
    expect(captured?.state).toBe('function_execute');
    expect(captured?.awaiting_approval).toEqual([]);
  });

  it('wraps handler failures as transition errors tagged with the from-state', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'steering_check' };
    vi.spyOn(persistence, 'loadRecord').mockResolvedValue(rec);
    const saveRecord = vi.spyOn(persistence, 'saveRecord').mockResolvedValue();
    const handle = vi.fn(async () => {
      throw new Error('boom');
    });

    await expect(
      runTransition({} as ISdk, 'steering_check', handle, { session_id: 's1' }),
    ).rejects.toThrow('transition from steering_check failed: Error: boom');
    expect(saveRecord).not.toHaveBeenCalled();
  });
});
