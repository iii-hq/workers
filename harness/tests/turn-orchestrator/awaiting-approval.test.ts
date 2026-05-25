import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { PreparedCall, TurnStateRecord, TurnWork } from '../../src/turn-orchestrator/state.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/function-awaiting-approval/process.js';

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

function workWith(prepared: PreparedCall[]): TurnWork {
  return { prepared, executed: {} };
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
        { route: 'dispatch', call: { id: 'fc-1', function_id: 'shell::run', arguments: {} } },
      ]),
    );
    await handleAwaitingApproval(iii, rec);
    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
    expect(rec.work?.prepared[0]?.route).toBe('dispatch');
  });

  it('folds pre_approved route into work.prepared on allow and transitions to function_execute', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'allow', reason: null };
      return null;
    });
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: { command: 'ls' } }],
      workWith([
        {
          route: 'dispatch',
          call: { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
        },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.work?.prepared[0]?.route).toBe('pre_approved');
  });

  it('sets synthetic denial result in work.prepared on deny and transitions to function_execute', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'deny', reason: 'policy' };
      return null;
    });
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }],
      workWith([
        { route: 'dispatch', call: { id: 'fc-1', function_id: 'shell::run', arguments: {} } },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    const entry = rec.work?.prepared[0];
    expect(entry?.route).toBe('synthetic');
    if (entry?.route === 'synthetic') {
      expect(entry.result.details).toMatchObject({
        approval_denied: true,
        decision: 'deny',
        reason: 'policy',
      });
    }
  });

  it('handles aborted decision like deny (folded into work.prepared)', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'aborted', reason: 'session_aborted' };
      return null;
    });
    const rec = recordWith(
      [{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }],
      workWith([
        { route: 'dispatch', call: { id: 'fc-1', function_id: 'shell::run', arguments: {} } },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    const entry = rec.work?.prepared[0];
    expect(entry?.route).toBe('synthetic');
    if (entry?.route === 'synthetic') {
      expect(entry.result.details).toMatchObject({ decision: 'aborted' });
    }
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
        { route: 'dispatch', call: { id: 'fc-1', function_id: 'shell::run', arguments: {} } },
        {
          route: 'dispatch',
          call: { id: 'fc-2', function_id: 'shell::fs::write', arguments: {} },
        },
      ]),
    );

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.work?.prepared[0]?.route).toBe('pre_approved');
    expect(rec.work?.prepared[1]?.route).toBe('synthetic');
  });
});
