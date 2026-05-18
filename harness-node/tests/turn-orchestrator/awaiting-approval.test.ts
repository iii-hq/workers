import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/states/functions.js';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';

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
): TurnStateRecord {
  return {
    session_id: 's1',
    state: 'function_awaiting_approval',
    turn_count: 0,
    max_turns: undefined,
    last_assistant: null,
    pending_function_calls: [],
    function_results: [],
    turn_end_emitted: false,
    started_at_ms: 0,
    updated_at_ms: 0,
    awaiting_approval: awaiting,
  };
}

describe('handleAwaitingApproval', () => {
  it('no-ops when any decision is missing', async () => {
    const iii = fakeIii((_scope, _key) => null);
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }]);
    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
    ]);
    await handleAwaitingApproval(iii, rec);
    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
  });

  it('marks prepared entries pre_approved on allow and transitions to function_execute', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'allow', reason: null };
      return null;
    });
    const rec = recordWith([
      { function_call_id: 'fc-1', function_id: 'shell::run', args: { command: 'ls' } },
    ]);
    const savedPrepared = vi.spyOn(persistence, 'savePreparedCalls').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      {
        function_call: {
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: { command: 'ls' },
        },
        blocked: null,
      },
    ]);

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.awaiting_approval).toEqual([]);
    const savedArg = savedPrepared.mock.calls[0][2];
    expect(savedArg[0].pre_approved).toBe(true);
    expect(savedArg[0].blocked).toBeNull();
  });

  it('sets blocked denial result on deny and transitions to function_execute', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'deny', reason: 'policy' };
      return null;
    });
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }]);
    const savedPrepared = vi.spyOn(persistence, 'savePreparedCalls').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
    ]);

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    const savedArg = savedPrepared.mock.calls[0][2];
    expect(savedArg[0].pre_approved).toBeFalsy();
    expect(savedArg[0].blocked).toMatchObject({
      details: expect.objectContaining({
        approval_denied: true,
        decision: 'deny',
        reason: 'policy',
      }),
    });
  });

  it('handles aborted decision like deny', async () => {
    const iii = fakeIii((_scope, key) => {
      if (key === 's1/fc-1') return { decision: 'aborted', reason: 'session_aborted' };
      return null;
    });
    const rec = recordWith([{ function_call_id: 'fc-1', function_id: 'shell::run', args: {} }]);
    const savedPrepared = vi.spyOn(persistence, 'savePreparedCalls').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      { function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} }, blocked: null },
    ]);

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    const savedArg = savedPrepared.mock.calls[0][2];
    expect(savedArg[0].blocked?.details).toMatchObject({ decision: 'aborted' });
  });
});
