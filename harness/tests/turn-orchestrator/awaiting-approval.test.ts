import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { PreparedEntry, TurnStateRecord, TurnWork } from '../../src/turn-orchestrator/state.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/states/function-awaiting-approval.js';

function fakeIii(stateGetImpl: (scope: string, key: string) => unknown): ISdk {
  return {
    trigger: vi.fn(async ({ function_id, payload }: { function_id: string; payload: unknown }) => {
      if (function_id === 'state::get') {
        const p = payload as { scope: string; key: string };
        return stateGetImpl(p.scope, p.key);
      }
      if (function_id === 'state::set') return null;
      return null;
    }),
  } as unknown as ISdk;
}

function recordWith(
  awaiting: { function_call_id: string; function_id: string; args: unknown }[],
  work?: TurnWork,
): TurnStateRecord {
  return {
    session_id: 's1',
    state: 'function_awaiting_approval',
    turn_count: 0,
    max_turns: undefined,
    last_assistant: null,
    function_results: [],
    turn_end_emitted: false,
    started_at_ms: 0,
    updated_at_ms: 0,
    awaiting_approval: awaiting,
    work,
  };
}

function workWith(batch: PreparedEntry[]): TurnWork {
  return { batch, results: [] };
}

describe('handleAwaitingApproval', () => {
  it('transitions straight to function_execute when awaiting is empty', async () => {
    const iii = fakeIii((_scope, _key) => null);
    const rec = recordWith([]);
    await handleAwaitingApproval(iii, rec);
    expect(rec.state).toBe('function_execute');
  });

  it('no-ops when any decision is missing', async () => {
    const iii = fakeIii((_scope, _key) => null);
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }],
      workWith([
        { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
      ]),
    );
    await handleAwaitingApproval(iii, rec);
    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
    // batch unchanged — no decision folded in
    expect(rec.work?.batch[0]?.pre_approved).toBeUndefined();
  });

  it('folds pre_approved into work.batch on allow and transitions to function_execute', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'allow', reason: null };
      return null;
    });
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: { command: 'ls' } }],
      workWith([
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
          blocked: null,
        },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.work?.batch[0]?.pre_approved).toBe(true);
    expect(rec.work?.batch[0]?.blocked).toBeNull();
  });

  it('sets blocked denial result in work.batch on deny and transitions to function_execute', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'deny', reason: 'policy' };
      return null;
    });
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }],
      workWith([
        { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.work?.batch[0]?.pre_approved).toBeFalsy();
    expect(rec.work?.batch[0]?.blocked).toMatchObject({
      details: expect.objectContaining({
        approval_denied: true,
        decision: 'deny',
        reason: 'policy',
      }),
    });
  });

  it('handles aborted decision like deny (folded into work.batch)', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'aborted', reason: 'session_aborted' };
      return null;
    });
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }],
      workWith([
        { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.work?.batch[0]?.pre_approved).toBeFalsy();
    expect(rec.work?.batch[0]?.blocked?.details).toMatchObject({ decision: 'aborted' });
  });

  it('folds independent decisions across a multi-call batch', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'allow', reason: null };
      if (key === 's1/fc-2') return { decision: 'deny', reason: 'policy' };
      return null;
    });
    const rec = recordWith(
      [
        { function_call_id: 'fc-1', function_id: 'shell::run', args: {} },
        { function_call_id: 'fc-2', function_id: 'shell::fs::write', args: {} },
      ],
      workWith([
        { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
        {
          function_call: { id: 'fc-2', function_id: 'shell::fs::write', arguments: {} },
          blocked: null,
        },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.work?.batch[0]?.pre_approved).toBe(true);
    expect(rec.work?.batch[0]?.blocked).toBeNull();
    expect(rec.work?.batch[1]?.pre_approved).toBeFalsy();
    expect(rec.work?.batch[1]?.blocked?.details).toMatchObject({ decision: 'deny' });
  });
});
