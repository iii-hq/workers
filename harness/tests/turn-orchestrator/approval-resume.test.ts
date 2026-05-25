import { afterEach, describe, expect, it, vi } from 'vitest';
import { TriggerAction, type ISdk } from '../../src/runtime/iii.js';
import { approvalResumeFnId } from '../../src/approval-gate/schemas.js';
import {
  clearApprovalResumeRegistry,
  recoverPendingApprovals,
  registerApprovalResume,
} from '../../src/turn-orchestrator/approval-resume.js';

type RegisteredFn = {
  fnId: string;
  handler: (payload: unknown) => Promise<unknown>;
  unregister: ReturnType<typeof vi.fn>;
};

import {
  newRecord,
  turnStateKey,
  type TurnStateRecord,
} from '../../src/turn-orchestrator/state.js';

function makeIiiWithRegistry(
  stateStore = new Map<string, unknown>(),
  agentTurnStates: TurnStateRecord[] = [],
) {
  const registered = new Map<string, RegisteredFn>();
  const wakeCalls: Array<{ session_id: string; action?: unknown; function_id?: string }> = [];

  const iii = {
    registerFunction: vi.fn((fnId: string, handler: (payload: unknown) => Promise<unknown>) => {
      const entry: RegisteredFn = {
        fnId,
        handler,
        unregister: vi.fn(),
      };
      registered.set(fnId, entry);
      return { unregister: entry.unregister };
    }),
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
          return stateStore.get(`${p.scope}/${p.key}`) ?? null;
        }
        if (function_id === 'state::set') {
          const p = payload as { scope: string; key: string; value: unknown };
          stateStore.set(`${p.scope}/${p.key}`, p.value);
          return null;
        }
        if (function_id === 'state::list') {
          return agentTurnStates;
        }
        if (function_id.startsWith('turn::') && function_id !== 'turn::on_abort_signal') {
          const p = payload as { session_id: string };
          wakeCalls.push({
            session_id: p.session_id,
            action,
            function_id,
          });
          return null;
        }
        return null;
      },
    ),
  } as unknown as ISdk;

  return { iii, registered, wakeCalls, stateStore };
}

afterEach(() => {
  clearApprovalResumeRegistry();
  vi.restoreAllMocks();
});

describe('registerApprovalResume', () => {
  it('registers turn::approval_resume::s1/fc-1 on first call', () => {
    const { iii, registered } = makeIiiWithRegistry();
    registerApprovalResume(iii, 's1', 'fc-1');
    expect(iii.registerFunction).toHaveBeenCalledWith(
      'turn::approval_resume::s1/fc-1',
      expect.any(Function),
      expect.objectContaining({ description: expect.any(String) }),
    );
    expect(registered.has('turn::approval_resume::s1/fc-1')).toBe(true);
    expect(approvalResumeFnId('s1', 'fc-1')).toBe('turn::approval_resume::s1/fc-1');
  });

  it('returns the same ref when registered twice', () => {
    const { iii, registered } = makeIiiWithRegistry();
    const a = registerApprovalResume(iii, 's1', 'fc-1');
    const b = registerApprovalResume(iii, 's1', 'fc-1');
    expect(a).toBe(b);
    expect(iii.registerFunction).toHaveBeenCalledTimes(1);
    expect(registered.size).toBe(1);
  });
});

describe('approval resume handler', () => {
  it('persists decision, enqueues turn::{state}, and unregisters', async () => {
    const { iii, registered, wakeCalls, stateStore } = makeIiiWithRegistry();
    const rec = newRecord('s1');
    rec.state = 'function_awaiting_approval';
    stateStore.set(`agent/${turnStateKey('s1')}`, rec);
    registerApprovalResume(iii, 's1', 'fc-1');
    const entry = registered.get('turn::approval_resume::s1/fc-1');
    if (!entry) throw new Error('handler not registered');
    await entry.handler({ decision: 'allow', reason: null });

    expect(stateStore.get('approvals/s1/fc-1')).toEqual({ decision: 'allow', reason: null });
    expect(wakeCalls).toEqual([
      {
        session_id: 's1',
        function_id: 'turn::function_awaiting_approval',
        action: TriggerAction.Enqueue({ queue: 'turn-step' }),
      },
    ]);
    expect(entry!.unregister).toHaveBeenCalled();
  });

  it('does not overwrite an existing decision (idempotent persist)', async () => {
    const { iii, registered, stateStore } = makeIiiWithRegistry();
    stateStore.set('approvals/s1/fc-1', { decision: 'aborted', reason: 'session_aborted' });
    registerApprovalResume(iii, 's1', 'fc-1');
    const entry = registered.get('turn::approval_resume::s1/fc-1');
    if (!entry) throw new Error('handler not registered');
    await entry.handler({
      decision: 'allow',
      reason: null,
    });
    expect(stateStore.get('approvals/s1/fc-1')).toEqual({
      decision: 'aborted',
      reason: 'session_aborted',
    });
  });

  it('does not enqueue turn::{state} again after unregister on second invoke', async () => {
    const { iii, registered, wakeCalls } = makeIiiWithRegistry();
    registerApprovalResume(iii, 's1', 'fc-1');
    const entry = registered.get('turn::approval_resume::s1/fc-1');
    if (!entry) throw new Error('handler not registered');
    await entry.handler({ decision: 'deny', reason: 'nope' });
    wakeCalls.length = 0;

    await entry.handler({ decision: 'allow', reason: null });
    expect(wakeCalls).toHaveLength(0);
  });
});

describe('recoverPendingApprovals', () => {
  it('re-registers resume fns for sessions in function_awaiting_approval', async () => {
    const { iii, registered } = makeIiiWithRegistry(new Map(), [
      {
        session_id: 's1',
        state: 'function_awaiting_approval',
        turn_count: 0,
        function_results: [],
        turn_end_emitted: false,
        started_at_ms: 0,
        updated_at_ms: 0,
        awaiting_approval: [
          { function_call_id: 'fc-1', function_id: 'tool::x', args: {} },
          { function_call_id: 'fc-2', function_id: 'tool::y', args: {} },
        ],
      },
      {
        session_id: 's2',
        state: 'stopped',
        turn_count: 0,
        function_results: [],
        turn_end_emitted: false,
        started_at_ms: 0,
        updated_at_ms: 0,
      },
    ]);
    await recoverPendingApprovals(iii);
    expect(registered.has('turn::approval_resume::s1/fc-1')).toBe(true);
    expect(registered.has('turn::approval_resume::s1/fc-2')).toBe(true);
    expect(registered.has('turn::approval_resume::s2/fc-1')).toBe(false);
  });

  it('ignores non-turn_state agent scope values (null, messages, etc.)', async () => {
    const { iii, registered } = makeIiiWithRegistry(new Map(), [
      null,
      { messages: [] },
      {
        session_id: 's1',
        state: 'function_awaiting_approval',
        turn_count: 0,
        function_results: [],
        turn_end_emitted: false,
        started_at_ms: 0,
        updated_at_ms: 0,
        awaiting_approval: [{ function_call_id: 'fc-1', function_id: 'tool::x', args: {} }],
      },
    ]);
    await recoverPendingApprovals(iii);
    expect(registered.has('turn::approval_resume::s1/fc-1')).toBe(true);
    expect(registered.size).toBe(1);
  });
});
