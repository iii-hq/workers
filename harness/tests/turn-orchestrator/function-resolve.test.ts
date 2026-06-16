import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import { handleFunctionResolve } from '../../src/turn-orchestrator/function-resolve.js';
import {
  TURN_STATE_SCOPE,
  newRecord,
  type TurnStateRecord,
} from '../../src/turn-orchestrator/state.js';

type Wake = { function_id: string; session_id: string };

function fixture(): {
  iii: ISdk;
  stateStore: Map<string, unknown>;
  wakes: Wake[];
  seedParked(session_id: string, callIds: string[]): TurnStateRecord;
} {
  const stateStore = new Map<string, unknown>();
  const wakes: Wake[] = [];
  const iii = {
    trigger: vi.fn(
      async ({
        function_id,
        payload,
        action,
      }: {
        function_id: string;
        payload: unknown;
        action?: unknown;
      }) => {
        if (function_id === 'state::get') {
          const p = payload as { scope: string; key: string };
          const v = stateStore.get(`${p.scope}/${p.key}`);
          return v === undefined ? null : structuredClone(v);
        }
        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          const key = `${p.scope}/${p.key}`;
          const old_value = stateStore.has(key) ? structuredClone(stateStore.get(key)) : null;
          stateStore.set(key, structuredClone(p.value));
          return { old_value, new_value: p.value };
        }
        if (function_id === 'state::delete') {
          const p = payload as { scope: string; key: string };
          stateStore.delete(`${p.scope}/${p.key}`);
          return {};
        }
        if (function_id.startsWith('turn::') && action != null) {
          wakes.push({ function_id, session_id: (payload as { session_id: string }).session_id });
          return null;
        }
        return null;
      },
    ),
  } as unknown as ISdk;

  function seedParked(session_id: string, callIds: string[]): TurnStateRecord {
    const rec = newRecord(session_id);
    rec.state = 'function_awaiting_approval';
    rec.last_assistant = null;
    rec.work = {
      prepared: callIds.map((id) => ({
        route: 'trigger' as const,
        call: { id, function_id: 'shell::run', arguments: {} },
      })),
      executed: {},
    };
    rec.awaiting_approval = callIds.map((id) => ({
      function_call_id: id,
      function_id: 'shell::run',
      args: {},
    }));
    stateStore.set(`${TURN_STATE_SCOPE}/${session_id}`, structuredClone(rec));
    return rec;
  }

  return { iii, stateStore, wakes, seedParked };
}

describe('harness::function::resolve', () => {
  it('execute persists the resolution row and wakes the parked turn', async () => {
    const f = fixture();
    const rec = f.seedParked('s1', ['fc-1']);

    const out = await handleFunctionResolve(f.iii, {
      session_id: 's1',
      turn_id: rec.turn_id,
      function_call_id: 'fc-1',
      action: 'execute',
    });

    expect(out).toEqual({ resolved: true, turn_resumed: true });
    expect(f.stateStore.get('function_resolutions/s1/fc-1')).toMatchObject({
      turn_id: rec.turn_id,
      action: 'execute',
    });
    expect(f.wakes).toEqual([
      { function_id: 'turn::function_awaiting_approval', session_id: 's1' },
    ]);
  });

  it('deliver persists content/details/is_error verbatim', async () => {
    const f = fixture();
    const rec = f.seedParked('s1', ['fc-1']);

    const out = await handleFunctionResolve(f.iii, {
      session_id: 's1',
      turn_id: rec.turn_id,
      function_call_id: 'fc-1',
      action: 'deliver',
      is_error: true,
      content: [{ type: 'text', text: 'too risky' }],
      details: { status: 'denied', denied_by: 'user', reason: 'too risky' },
    });

    expect(out.resolved).toBe(true);
    expect(f.stateStore.get('function_resolutions/s1/fc-1')).toMatchObject({
      action: 'deliver',
      is_error: true,
      content: [{ type: 'text', text: 'too risky' }],
      details: { denied_by: 'user' },
    });
  });

  it('deliver without content is rejected', async () => {
    const f = fixture();
    const rec = f.seedParked('s1', ['fc-1']);

    await expect(
      handleFunctionResolve(f.iii, {
        session_id: 's1',
        turn_id: rec.turn_id,
        function_call_id: 'fc-1',
        action: 'deliver',
        is_error: true,
      }),
    ).rejects.toThrow(/content is required/);
  });

  it('rejects "/" in ids at the boundary', async () => {
    const f = fixture();
    await expect(
      handleFunctionResolve(f.iii, {
        session_id: 's/1',
        turn_id: 't_1',
        function_call_id: 'fc-1',
        action: 'execute',
      }),
    ).rejects.toThrow(/must not contain/);
  });

  it.each([
    ['unknown session', 's-missing', 't_any', 'fc-1'],
    ['unknown call', 's1', undefined, 'fc-unknown'],
  ] as const)('%s answers {resolved: false}', async (_label, session_id, turn_id, call_id) => {
    const f = fixture();
    const rec = f.seedParked('s1', ['fc-1']);

    const out = await handleFunctionResolve(f.iii, {
      session_id,
      turn_id: turn_id ?? rec.turn_id,
      function_call_id: call_id,
      action: 'execute',
    });
    expect(out).toEqual({ resolved: false });
    expect(f.wakes).toEqual([]);
  });

  it('a stale turn_id answers {resolved: false}', async () => {
    const f = fixture();
    f.seedParked('s1', ['fc-1']);

    const out = await handleFunctionResolve(f.iii, {
      session_id: 's1',
      turn_id: 't_previous_run',
      function_call_id: 'fc-1',
      action: 'execute',
    });
    expect(out).toEqual({ resolved: false });
  });

  it('a record outside the function-batch states answers {resolved: false}', async () => {
    const f = fixture();
    const rec = f.seedParked('s1', ['fc-1']);
    rec.state = 'assistant_streaming';
    f.stateStore.set(`${TURN_STATE_SCOPE}/s1`, structuredClone(rec));

    const out = await handleFunctionResolve(f.iii, {
      session_id: 's1',
      turn_id: rec.turn_id,
      function_call_id: 'fc-1',
      action: 'execute',
    });
    expect(out).toEqual({ resolved: false });
  });

  it('duplicate resolves keep the first decision and re-enqueue the wake', async () => {
    const f = fixture();
    const rec = f.seedParked('s1', ['fc-1']);

    await handleFunctionResolve(f.iii, {
      session_id: 's1',
      turn_id: rec.turn_id,
      function_call_id: 'fc-1',
      action: 'execute',
    });
    const out = await handleFunctionResolve(f.iii, {
      session_id: 's1',
      turn_id: rec.turn_id,
      function_call_id: 'fc-1',
      action: 'deliver',
      is_error: true,
      content: [{ type: 'text', text: 'late deny' }],
    });

    expect(out).toEqual({ resolved: true, turn_resumed: true });
    // First decision wins — the deliver did not overwrite the execute.
    expect(f.stateStore.get('function_resolutions/s1/fc-1')).toMatchObject({ action: 'execute' });
    expect(f.wakes).toHaveLength(2);
  });

  it('a state read failure surfaces as an error, never {resolved: false}', async () => {
    const f = fixture();
    const trigger = f.iii.trigger as unknown as ReturnType<typeof vi.fn>;
    trigger.mockImplementation(async ({ function_id }: { function_id: string }) => {
      if (function_id === 'state::get') throw new Error('state down');
      return null;
    });

    await expect(
      handleFunctionResolve(f.iii, {
        session_id: 's1',
        turn_id: 't_1',
        function_call_id: 'fc-1',
        action: 'execute',
      }),
    ).rejects.toThrow('state down');
  });
});
