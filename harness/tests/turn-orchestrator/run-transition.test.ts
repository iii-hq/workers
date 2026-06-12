import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { CompactionBusyError, TransientError } from '../../src/turn-orchestrator/errors.js';
import { TURN_STATE_SCOPE } from '../../src/turn-orchestrator/state.js';
import { runTransition } from '../../src/turn-orchestrator/run-transition.js';
import {
  type TurnStateRecord,
  newRecord,
  transitionTo,
} from '../../src/turn-orchestrator/state.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';

afterEach(() => {
  vi.restoreAllMocks();
});

describe('runTransition', () => {
  it('throws when the session record is missing, without running the handler', async () => {
    installMockTurnStore({ loadRecord: vi.fn(async () => null) });
    const handle = vi.fn();

    await expect(
      runTransition({} as ISdk, 'provisioning', handle, { session_id: 'missing' }),
    ).rejects.toThrow('turn::provisioning invariant: missing session missing');
    expect(handle).not.toHaveBeenCalled();
  });

  it('returns a stale skip without running the handler or saving', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const store = installMockTurnStore({ loadRecord: vi.fn(async () => rec) });
    const handle = vi.fn();

    const result = await runTransition({} as ISdk, 'provisioning', handle, { session_id: 's1' });

    expect(result).toEqual({ ok: true, skipped: true, reason: 'stale' });
    expect(handle).not.toHaveBeenCalled();
    expect(store.saveRecord).not.toHaveBeenCalled();
  });

  it('runs the handler and threads the pre-mutation snapshot into saveRecord', async () => {
    const iii = {} as ISdk;
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'provisioning' };
    const store = installMockTurnStore({ loadRecord: vi.fn(async () => rec) });
    const handle = vi.fn(async (_iii: ISdk, r: TurnStateRecord) => {
      transitionTo(r, 'assistant_streaming');
    });

    const result = await runTransition(iii, 'provisioning', handle, { session_id: 's1' });

    expect(handle).toHaveBeenCalledWith(iii, rec);
    expect(store.saveRecord).toHaveBeenCalledWith(
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
    let captured: TurnStateRecord | null | undefined;
    installMockTurnStore({
      loadRecord: vi.fn(async () => rec),
      saveRecord: vi.fn(async (_r, previous) => {
        captured = previous;
      }),
    });
    const handle = vi.fn(async (_iii: ISdk, r: TurnStateRecord) => {
      r.awaiting_approval?.push({ function_call_id: 'fc-1', function_id: 'f', args: {} });
      transitionTo(r, 'function_awaiting_approval');
    });

    await runTransition(iii, 'function_execute', handle, { session_id: 's1' });

    expect(captured?.state).toBe('function_execute');
    expect(captured?.awaiting_approval).toEqual([]);
  });

  it('routes an unexpected handler throw to failed without re-throwing', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const store = installMockTurnStore({
      loadRecord: vi.fn(async () => rec),
    });
    const handle = vi.fn(async () => {
      throw new Error('boom');
    });

    const result = await runTransition({} as ISdk, 'assistant_streaming', handle, {
      session_id: 's1',
    });
    expect(result).toMatchObject({ ok: true, to_state: 'failed' });
    expect(store.saveRecord).toHaveBeenCalled();
  });
});

function fakeIii(record: unknown) {
  const writes: Array<{ function_id: string; payload: unknown }> = [];
  const iii = {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      writes.push({ function_id, payload });
      const p = payload as Record<string, unknown>;
      if (function_id === 'state::get' && p.scope === TURN_STATE_SCOPE && p.key === 's1') {
        return record;
      }
      return null;
    }),
  } as unknown as ISdk;
  return { iii, writes };
}

describe('runTransition error model', () => {
  const base = {
    session_id: 's1',
    state: 'function_execute',
    turn_count: 1,
    function_results: [],
    turn_end_emitted: false,
    started_at_ms: 1,
    updated_at_ms: 1,
  };

  it('routes an unexpected throw to failed and does not re-throw', async () => {
    const { iii, writes } = fakeIii({ ...base });
    const res = await runTransition(
      iii,
      'function_execute',
      async () => {
        throw new Error('boom');
      },
      { session_id: 's1' },
    );
    expect(res).toMatchObject({ ok: true, to_state: 'failed' });
    const saved = writes.find(
      (w) =>
        w.function_id === 'state::set' &&
        w.payload.scope === TURN_STATE_SCOPE &&
        w.payload.key === 's1',
    );
    expect(saved?.payload.value.state).toBe('failed');
    expect(saved?.payload.value.error.message).toContain('boom');
    const surfaced = writes.some(
      (w) =>
        w.function_id === 'stream::set' &&
        w.payload.data?.type === 'message_complete' &&
        w.payload.data?.message?.stop_reason === 'error',
    );
    expect(surfaced).toBe(true);
    const ended = writes.some(
      (w) => w.function_id === 'stream::set' && w.payload.data?.type === 'agent_end',
    );
    expect(ended).toBe(true);
  });

  it('re-throws TransientError so the queue retries', async () => {
    const { iii } = fakeIii({ ...base });
    await expect(
      runTransition(
        iii,
        'function_execute',
        async () => {
          throw new TransientError('retry me');
        },
        { session_id: 's1' },
      ),
    ).rejects.toThrow('retry me');
  });

  it('CompactionBusyError IS a TransientError — the subclass relation is the fix', () => {
    // A one-line revert of `extends TransientError` back to `extends Error`
    // resurrects the terminal-failure bug; pin the contract at the source.
    const err = new CompactionBusyError('compaction already in progress');
    expect(err).toBeInstanceOf(TransientError);
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe('CompactionBusyError');
  });

  it('re-throws CompactionBusyError (transient) instead of failing the session', async () => {
    // Regression: a busy compaction lease (async post-turn summarize of a
    // large session) used to route the turn to terminal `failed` with
    // "response failed: from assistant_streaming: compaction already in
    // progress". Busy is lease-TTL-bounded — the queue must retry instead.
    const { iii, writes } = fakeIii({ ...base, state: 'assistant_streaming' });
    await expect(
      runTransition(
        iii,
        'assistant_streaming',
        async () => {
          throw new CompactionBusyError('compaction already in progress');
        },
        { session_id: 's1' },
      ),
    ).rejects.toThrow('compaction already in progress');
    // The session record must NOT be routed to failed.
    const failedWrite = writes.find(
      (w) =>
        w.function_id === 'state::set' &&
        w.payload.scope === TURN_STATE_SCOPE &&
        w.payload.value?.state === 'failed',
    );
    expect(failedWrite).toBeUndefined();
  });
});
