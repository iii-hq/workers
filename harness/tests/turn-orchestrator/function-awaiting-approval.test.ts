import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as events from '../../src/turn-orchestrator/events.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import { applyDecisionToPrepared } from '../../src/turn-orchestrator/function-awaiting-approval/run.js';
import { handleAwaitingApproval } from '../../src/turn-orchestrator/function-awaiting-approval/process.js';
import { enterFunctionExecute } from '../../src/turn-orchestrator/function-execute/run.js';
import type { FunctionBatchWork } from '../../src/turn-orchestrator/function-execute/types.js';
import type { FunctionResolution } from '../../src/turn-orchestrator/function-resolve.js';
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

function executeResolution(turn_id = 't_1'): FunctionResolution {
  return { turn_id, action: 'execute', resolved_at: 1 };
}

function deliverResolution(
  overrides?: Partial<Extract<FunctionResolution, { action: 'deliver' }>>,
): FunctionResolution {
  return {
    turn_id: 't_1',
    action: 'deliver',
    is_error: true,
    content: [{ type: 'text', text: 'Permission denied by user: policy' }],
    details: { status: 'denied', denied_by: 'user', reason: 'policy' },
    resolved_at: 1,
    ...overrides,
  };
}

/** Mock bus: `function_resolutions` rows live in `resolutionStore`, keyed `<sid>/<cid>`. */
function makeIii(resolutionStore: Map<string, unknown>): ISdk {
  return {
    trigger: vi.fn(async (req: { function_id: string; payload: unknown }) => {
      if (req.function_id === 'state::get') {
        const p = req.payload as { scope: string; key: string };
        return resolutionStore.get(`${p.scope}/${p.key}`) ?? null;
      }
      if (req.function_id === 'state::delete') {
        const p = req.payload as { scope: string; key: string };
        resolutionStore.delete(`${p.scope}/${p.key}`);
        return {};
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
  const triggerCall = {
    route: 'trigger' as const,
    call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
  };

  it('maps execute to pre_approved', () => {
    expect(applyDecisionToPrepared(triggerCall, executeResolution())).toEqual({
      route: 'pre_approved',
      call: triggerCall.call,
    });
  });

  it('maps deliver to a synthetic result carrying the delivered content verbatim', () => {
    const resolution = deliverResolution();
    const resolved = applyDecisionToPrepared(triggerCall, resolution);
    expect(resolved).toEqual({
      route: 'synthetic',
      call: triggerCall.call,
      result: {
        content: [{ type: 'text', text: 'Permission denied by user: policy' }],
        details: { status: 'denied', denied_by: 'user', reason: 'policy' },
        terminate: false,
      },
      is_error: true,
    });
  });

  it('deliver carries is_error through (false stays false)', () => {
    const resolved = applyDecisionToPrepared(
      triggerCall,
      deliverResolution({ is_error: false, details: undefined }),
    );
    expect(resolved).toMatchObject({ route: 'synthetic', is_error: false });
    expect((resolved as { result: { details: unknown } }).result.details).toEqual({});
  });
});

describe('handleAwaitingApproval', () => {
  it('executes a released call and finalizes when the batch completes', async () => {
    const resolutionStore = new Map<string, unknown>();
    const rec = newRecord('s1');
    resolutionStore.set('function_resolutions/s1/fc-1', executeResolution(rec.turn_id));
    const iii = makeIii(resolutionStore);
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } };
    seedFunctionAwaitingApproval(
      rec,
      { prepared: [{ route: 'trigger', call: fc }], executed: {} },
      [{ function_call_id: 'fc-1', function_id: 'shell::run' }],
    );

    installMockTurnStore({
      appendMessages: vi.fn(async () => {}),
    });
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleAwaitingApproval(iii, rec);

    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.state).toBe('assistant_streaming');
    expect(rec.work).toBeUndefined();
    expect(rec.function_results).toEqual([]);
    // Consumed resolution row is deleted.
    expect(resolutionStore.has('function_resolutions/s1/fc-1')).toBe(false);
    const turnEnd = emitSpy.mock.calls.find((call) => call[2]?.type === 'turn_end')?.[2] as
      | { function_results: unknown[] }
      | undefined;
    expect(turnEnd?.function_results).toHaveLength(1);
  });

  it('crash between checkpoint and row delete: the replay wake cleans up without re-executing', async () => {
    // Simulates: a prior wake executed fc-1 and checkpointed, then crashed
    // before deleting the resolution row and before the awaiting/route save.
    const resolutionStore = new Map<string, unknown>();
    const rec = newRecord('s1');
    resolutionStore.set('function_resolutions/s1/fc-1', executeResolution(rec.turn_id));
    const iii = makeIii(resolutionStore);
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionAwaitingApproval(
      rec,
      {
        prepared: [{ route: 'trigger', call: fc }],
        executed: {
          'fc-1': {
            call: fc,
            result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
            is_error: false,
            duration_ms: 1,
          },
        },
      },
      [{ function_call_id: 'fc-1', function_id: 'shell::run' }],
    );
    installMockTurnStore({
      loadMessages: vi.fn(async () => []),
      appendMessages: vi.fn(async () => {}),
    });
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleAwaitingApproval(iii, rec);

    // No second execution: the target was never triggered again.
    const trigger = iii.trigger as unknown as ReturnType<typeof vi.fn>;
    const shellCalls = trigger.mock.calls.filter(
      (call: [{ function_id: string }]) => call[0].function_id === 'shell::run',
    );
    expect(shellCalls).toHaveLength(0);
    expect(
      emitSpy.mock.calls.filter((call) => call[2]?.type === 'function_execution_end'),
    ).toHaveLength(0);
    // The orphaned row is collected and the batch finalizes.
    expect(resolutionStore.has('function_resolutions/s1/fc-1')).toBe(false);
    expect(rec.awaiting_approval).toEqual([]);
    expect(rec.state).toBe('assistant_streaming');
  });

  it('leaves state parked when awaiting entries remain unresolved', async () => {
    const iii = makeIii(new Map());
    const rec = newRecord('s1');
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionAwaitingApproval(
      rec,
      { prepared: [{ route: 'trigger', call: fc }], executed: {} },
      [{ function_call_id: 'fc-1', function_id: 'shell::run' }],
    );
    installMockTurnStore();

    await handleAwaitingApproval(iii, rec);

    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
  });

  it('returns to function_execute when resolutions are drained but the batch is incomplete', async () => {
    const resolutionStore = new Map<string, unknown>();
    const rec = newRecord('s1');
    resolutionStore.set('function_resolutions/s1/fc-2', deliverResolution());
    const iii = makeIii(resolutionStore);
    const fc1 = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const fc2 = { id: 'fc-2', function_id: 'shell::run', arguments: {} };
    const fc3 = { id: 'fc-3', function_id: 'shell::run', arguments: {} };
    seedFunctionAwaitingApproval(
      rec,
      {
        prepared: [
          { route: 'trigger', call: fc1 },
          { route: 'trigger', call: fc2 },
          { route: 'trigger', call: fc3 },
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
    expect(rec.work?.executed['fc-2']?.is_error).toBe(true);
    expect(rec.work?.executed['fc-3']).toBeUndefined();
    expect(resolutionStore.has('function_resolutions/s1/fc-2')).toBe(false);
  });
});
