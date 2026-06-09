import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as events from '../../src/turn-orchestrator/events.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import {
  applyDecisionToPrepared,
  denialResultFromDecision,
} from '../../src/turn-orchestrator/function-awaiting-approval/run.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/function-awaiting-approval/process.js';
import { enterFunctionExecute } from '../../src/turn-orchestrator/function-execute/run.js';
import type { FunctionBatchWork } from '../../src/turn-orchestrator/function-execute/types.js';
import { newRecord, type TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

afterEach(() => {
  vi.restoreAllMocks();
});

function makeAssistant(
  calls: Array<{ id: string; function_id: string; arguments?: unknown }> = [],
): AssistantMessage {
  return {
    role: 'assistant',
    content: calls.map((c) => ({
      type: 'function_call' as const,
      id: c.id,
      function_id: c.function_id,
      arguments: c.arguments ?? {},
    })),
    stop_reason: 'function_call',
    error_message: null,
    error_kind: null,
    usage: null,
    model: 'm',
    provider: 'p',
    timestamp: 1,
  };
}

function seedFunctionAwaitingApproval(
  rec: TurnStateRecord,
  work: FunctionBatchWork,
  awaiting: Array<{ function_call_id: string; function_id: string; args?: unknown }>,
  asst?: AssistantMessage,
): void {
  enterFunctionExecute(rec, asst ?? makeAssistant());
  rec.work = work;
  rec.awaiting_approval = awaiting.map((e) => ({
    function_call_id: e.function_call_id,
    function_id: e.function_id,
    args: e.args ?? {},
  }));
  rec.state = 'function_awaiting_approval';
}

function makeIii(approvalStore: Map<string, unknown>): ISdk {
  return {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      if (req.function_id === 'state::get') {
        const p = req.payload as { scope: string; key: string };
        return approvalStore.get(`${p.scope}/${p.key}`) ?? null;
      }
      if (req.function_id === 'state::update') return { old_value: 0 };
      if (req.function_id === 'stream::set') return null;
      if (req.function_id === 'shell::run') {
        return {
          content: [{ type: 'text' as const, text: 'ok' }],
          details: {},
          terminate: false,
        };
      }
      return null;
    }),
  } as unknown as ISdk;
}

describe('applyDecisionToPrepared', () => {
  const dispatchCall = {
    route: 'dispatch' as const,
    call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
  };

  it('maps allow to pre_approved', () => {
    expect(applyDecisionToPrepared(dispatchCall, { decision: 'allow', reason: null })).toEqual({
      route: 'pre_approved',
      call: dispatchCall.call,
    });
  });

  it('maps deny to synthetic denial result', () => {
    const resolved = applyDecisionToPrepared(dispatchCall, { decision: 'deny', reason: 'policy' });
    expect(resolved.route).toBe('synthetic');
    expect(resolved).toMatchObject({
      result: denialResultFromDecision({ decision: 'deny', reason: 'policy' }),
    });
  });

  it('a denial resumes the loop; an abort terminates the turn', () => {
    expect(denialResultFromDecision({ decision: 'deny', reason: null }).terminate).toBe(false);
    expect(denialResultFromDecision({ decision: 'aborted', reason: null }).terminate).toBe(true);
  });
});

describe('handleAwaitingApproval', () => {
  it('executes allow decision and finalizes when batch completes', async () => {
    const approvalStore = new Map<string, unknown>();
    approvalStore.set('approvals/s1/fc-1', { decision: 'allow', reason: null });
    const iii = makeIii(approvalStore);
    const rec = newRecord('s1');
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } };
    seedFunctionAwaitingApproval(
      rec,
      { prepared: [{ route: 'dispatch', call: fc }], executed: {} },
      [{ function_call_id: 'fc-1', function_id: 'shell::run' }],
    );

    installMockTurnStore({
      loadMessages: vi.fn(async () => []),
      appendMessages: vi.fn(async () => {}),
    });
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleAwaitingApproval(iii, rec);

    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.state).toBe('assistant_streaming');
    expect(rec.work).toBeUndefined();
    // Results travel on turn_end; the inline resume clears the record.
    expect(rec.function_results).toEqual([]);
    const turnEnd = emitSpy.mock.calls.find((call) => call[2]?.type === 'turn_end')?.[2] as
      | { function_results: unknown[] }
      | undefined;
    expect(turnEnd?.function_results).toHaveLength(1);
  });

  it('leaves state parked when awaiting entries remain undecided', async () => {
    const iii = makeIii(new Map());
    const rec = newRecord('s1');
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionAwaitingApproval(
      rec,
      { prepared: [{ route: 'dispatch', call: fc }], executed: {} },
      [{ function_call_id: 'fc-1', function_id: 'shell::run' }],
    );
    installMockTurnStore();

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
  });

  it('returns to function_execute when approvals done but batch incomplete', async () => {
    const approvalStore = new Map<string, unknown>();
    approvalStore.set('approvals/s1/fc-2', { decision: 'deny', reason: null });
    const iii = makeIii(approvalStore);
    const rec = newRecord('s1');
    const fc1 = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const fc2 = { id: 'fc-2', function_id: 'shell::run', arguments: {} };
    const fc3 = { id: 'fc-3', function_id: 'shell::run', arguments: {} };
    seedFunctionAwaitingApproval(
      rec,
      {
        prepared: [
          { route: 'dispatch', call: fc1 },
          { route: 'dispatch', call: fc2 },
          { route: 'dispatch', call: fc3 },
        ],
        executed: {
          'fc-1': {
            call: fc1,
            result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
            is_error: false,
            duration_ms: 1,
          },
        },
      },
      [{ function_call_id: 'fc-2', function_id: 'shell::run' }],
    );
    installMockTurnStore();
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.work?.executed['fc-2']).toBeDefined();
    expect(rec.work?.executed['fc-3']).toBeUndefined();
  });
});
