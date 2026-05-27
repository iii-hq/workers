import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as events from '../../src/turn-orchestrator/events.js';
import * as hookModule from '../../src/turn-orchestrator/hook.js';
import { agentTriggerCall, makeAssistant } from './_helpers/builders.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import * as agentTriggerModule from '../../src/turn-orchestrator/agent-trigger.js';
import { parseApprovalDecision } from '../../src/turn-orchestrator/function-awaiting-approval/ports.js';
import { handleExecute } from '../../src/turn-orchestrator/function-execute/process.js';
import { enterFunctionExecute } from '../../src/turn-orchestrator/function-execute/run.js';
import type { FunctionBatchWork } from '../../src/turn-orchestrator/function-execute/types.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

afterEach(() => {
  vi.restoreAllMocks();
});

function mockFinalizePersistence(): void {
  installMockTurnStore({
    loadMessages: vi.fn(async () => []),
    appendMessages: vi.fn(async () => {}),
  });
}

describe('parseApprovalDecision', () => {
  it('accepts allow/deny/aborted with nullable reason (stored approval shape)', () => {
    expect(parseApprovalDecision({ decision: 'allow', reason: null })).toEqual({
      decision: 'allow',
      reason: null,
    });
    expect(parseApprovalDecision({ decision: 'deny', reason: 'policy' })).toEqual({
      decision: 'deny',
      reason: 'policy',
    });
    expect(parseApprovalDecision({ decision: 'aborted', reason: 'session_aborted' })).toEqual({
      decision: 'aborted',
      reason: 'session_aborted',
    });
  });

  it('rejects speculative wrapper envelopes no caller stores', () => {
    expect(parseApprovalDecision({ data: { decision: 'allow', reason: null } })).toBeNull();
    expect(parseApprovalDecision({ payload: { decision: 'allow', reason: null } })).toBeNull();
  });

  it.each([
    ['null', null],
    ['undefined', undefined],
    ['missing decision', { reason: null }],
    ['empty decision', { decision: '', reason: null }],
    ['unknown decision', { decision: 'needs_approval', reason: null }],
    ['numeric reason', { decision: 'allow', reason: 7 }],
  ] as const)('rejects bad shape: %s', (_label, value) => {
    expect(parseApprovalDecision(value)).toBeNull();
  });
});

/** Seed required function-batch invariants before handleExecute. */
function seedFunctionExecute(
  rec: TurnStateRecord,
  work: FunctionBatchWork,
  asst?: AssistantMessage,
): void {
  enterFunctionExecute(rec, asst ?? makeAssistant([]));
  rec.work = work;
  rec.state = 'function_execute';
}

describe('handleExecute new flow', () => {
  it('runs the prepared batch from work', async () => {
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({
      kind: 'result',
      result: {
        content: [{ type: 'text' as const, text: 'ok' }],
        details: {},
        terminate: false,
      },
    });
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    enterFunctionExecute(
      rec,
      makeAssistant([agentTriggerCall('fc-1', 'shell::run', { command: 'ls' })]),
    );
    rec.state = 'function_execute';

    mockFinalizePersistence();
    await handleExecute(iii, rec);
    expect(rec.state).toBe('steering_check');
    expect(rec.function_results).toHaveLength(1);
    expect(rec.function_results[0]?.function_call_id).toBe('fc-1');
  });

  it('finishes the session when every function result terminates', async () => {
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionExecute(rec, {
      prepared: [{ route: 'dispatch', call: fc }],
      executed: {
        'fc-1': {
          call: fc,
          result: {
            content: [{ type: 'text' as const, text: 'bye' }],
            details: {},
            terminate: true,
          },
          is_error: false,
          duration_ms: 1,
        },
      },
    });
    mockFinalizePersistence();

    await handleExecute(iii, rec);

    expect(rec.state).toBe('stopped');
  });

  it('does not re-emit function_execution_start for already-executed calls on re-entry', async () => {
    const emitted: Array<{ type: string; function_call_id?: string }> = [];
    vi.spyOn(events, 'emit').mockImplementation(async (_iii, _sid, ev: never) => {
      emitted.push(ev as { type: string; function_call_id?: string });
    });
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({
      kind: 'result',
      result: { content: [{ type: 'text' as const, text: 'ok' }], details: {}, terminate: false },
    });
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    const fc1 = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const fc2 = { id: 'fc-2', function_id: 'shell::run', arguments: {} };
    seedFunctionExecute(rec, {
      prepared: [
        { route: 'dispatch', call: fc1 },
        { route: 'dispatch', call: fc2 },
      ],
      executed: {
        'fc-1': {
          call: fc1,
          result: {
            content: [{ type: 'text' as const, text: 'done' }],
            details: {},
            terminate: false,
          },
          is_error: false,
          duration_ms: 5,
        },
      },
    });
    mockFinalizePersistence();

    await handleExecute(iii, rec);

    const starts = emitted
      .filter((e) => e.type === 'function_execution_start')
      .map((e) => e.function_call_id);
    expect(starts).toEqual(['fc-2']);
    const fc1Ends = emitted.filter(
      (e) => e.type === 'function_execution_end' && e.function_call_id === 'fc-1',
    );
    expect(fc1Ends).toHaveLength(0);
  });

  it('skips consultBefore on pre_approved entries and uses triggerFunctionCall', async () => {
    const triggerSpy = vi.fn().mockResolvedValue({ ok: true });
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    seedFunctionExecute(rec, {
      prepared: [
        {
          route: 'pre_approved',
          call: { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
        },
      ],
      executed: {},
    });
    const consultBeforeSpy = vi.spyOn(hookModule, 'consultBefore');
    mockFinalizePersistence();

    await handleExecute(iii, rec);

    expect(consultBeforeSpy).not.toHaveBeenCalled();
    const triggerCalls = triggerSpy.mock.calls.map(
      (call) => (call[0] as { function_id: string }).function_id,
    );
    expect(triggerCalls).toContain('shell::run');
  });

  it('synthesizes a gate_unavailable denial when a pre_approved trigger rejects', async () => {
    const triggerSpy = vi.fn(async (req: { function_id: string }) => {
      if (req.function_id === 'shell::fs::write') {
        throw new Error('handler error: {"code":"S210","message":"bad write payload"}');
      }
      return null;
    });
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    seedFunctionExecute(rec, {
      prepared: [
        {
          route: 'pre_approved',
          call: {
            id: 'fc-1',
            function_id: 'shell::fs::write',
            arguments: { content: 'Tue May 19 08:17:10 -03 2026\n' },
          },
        },
      ],
      executed: {},
    });
    mockFinalizePersistence();

    await expect(handleExecute(iii, rec)).resolves.toBeUndefined();

    expect(rec.state).toBe('steering_check');
    expect(rec.function_results).toHaveLength(1);
    expect(rec.function_results[0]?.is_error).toBe(true);
    const details = rec.function_results[0]?.details as Record<string, unknown>;
    expect(details?.status).toBe('denied');
    expect(details?.denied_by).toBe('gate_unavailable');
    expect(details?.function_id).toBe('shell::fs::write');
    expect(String(details?.reason)).toContain('S210');
  });

  it('emits denial result without dispatching when route is synthetic', async () => {
    const triggerSpy = vi.fn().mockResolvedValue(null);
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    const denial = {
      content: [{ type: 'text' as const, text: 'denied' }],
      details: { approval_denied: true, decision: 'deny' as const },
      terminate: false,
    };
    seedFunctionExecute(rec, {
      prepared: [
        {
          route: 'synthetic',
          call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          result: denial,
        },
      ],
      executed: {},
    });
    mockFinalizePersistence();
    await handleExecute(iii, rec);

    const shellCalls = triggerSpy.mock.calls.filter(
      (call) => (call[0] as { function_id: string }).function_id === 'shell::run',
    );
    expect(shellCalls).toHaveLength(0);
    expect(rec.state).toBe('steering_check');
  });

  it('replays persisted executed calls without re-dispatching (re-entry with pre-populated work.executed)', async () => {
    const dispatchSpy = vi.spyOn(agentTriggerModule, 'dispatchWithHook');
    const triggerSpy = vi.fn().mockResolvedValue(null);
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec = newRecord('s1');
    const existingResult = {
      content: [{ type: 'text' as const, text: 'cached' }],
      details: {},
      terminate: false,
    };
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionExecute(rec, {
      prepared: [{ route: 'dispatch', call: fc }],
      executed: {
        'fc-1': {
          call: fc,
          result: existingResult,
          is_error: false,
          duration_ms: 42,
        },
      },
    });
    mockFinalizePersistence();

    await handleExecute(iii, rec);

    expect(dispatchSpy).not.toHaveBeenCalled();
    expect(rec.state).toBe('steering_check');
  });

  it('transitions to steering_check after a successful hook dispatch', async () => {
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValueOnce({
      kind: 'result',
      result: {
        content: [{ type: 'text' as const, text: 'ok' }],
        details: {},
        terminate: false,
      },
    });
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    enterFunctionExecute(rec, makeAssistant([agentTriggerCall('fc-1', 'shell::run')]));

    mockFinalizePersistence();
    await handleExecute(iii, rec);

    expect(rec.state).toBe('steering_check');
  });

  it('emits turn lifecycle and sets turn_end_emitted when last_assistant is present', async () => {
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec = newRecord('s1');
    const asst: AssistantMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: 'done' }],
      stop_reason: 'end',
      error_message: null,
      error_kind: null,
      usage: null,
      model: 'm',
      provider: 'p',
      timestamp: 1,
    };
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionExecute(
      rec,
      {
        prepared: [{ route: 'dispatch', call: fc }],
        executed: {
          'fc-1': {
            call: fc,
            result: {
              content: [{ type: 'text' as const, text: 'ok' }],
              details: {},
              terminate: false,
            },
            is_error: false,
            duration_ms: 1,
          },
        },
      },
      asst,
    );
    installMockTurnStore({
      loadMessages: vi.fn(async () => []),
      appendMessages: vi.fn(async () => {}),
    });
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleExecute(iii, rec);

    expect(rec.state).toBe('steering_check');
    expect(rec.turn_end_emitted).toBe(true);
    expect(emitSpy.mock.calls.some((call) => call[2]?.type === 'turn_end')).toBe(true);
  });

  it('does NOT duplicate function_results in flat-state when handleExecute re-enters', async () => {
    const existingResult = {
      content: [{ type: 'text' as const, text: 'ok' }],
      details: {},
      terminate: false,
    };
    const fc = { id: 'toolu_01', function_id: 'shell::run', arguments: { command: 'ls' } };

    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    enterFunctionExecute(
      rec,
      makeAssistant([agentTriggerCall('toolu_01', 'shell::run', { command: 'ls' })]),
    );

    let storedMessages: unknown[] = [];
    installMockTurnStore({
      loadMessages: vi.fn(async () => storedMessages as never),
      appendMessages: vi.fn(async (_sid, msgs) => {
        storedMessages = [...storedMessages, ...msgs];
      }),
    });
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValue({
      kind: 'result',
      result: existingResult,
    });

    await handleExecute(iii, rec);

    const asst = makeAssistant([agentTriggerCall('toolu_01', 'shell::run', { command: 'ls' })]);
    seedFunctionExecute(
      rec,
      {
        prepared: [{ route: 'dispatch', call: fc }],
        executed: {
          toolu_01: {
            call: fc,
            result: existingResult,
            is_error: false,
            duration_ms: 5,
          },
        },
      },
      asst,
    );
    rec.turn_end_emitted = false;

    await handleExecute(iii, rec);

    const fnResults = (
      storedMessages as Array<{ role?: string; function_call_id?: string }>
    ).filter((m) => m.role === 'function_result');
    expect(fnResults).toHaveLength(1);
    expect(fnResults[0]?.function_call_id).toBe('toolu_01');
  });
});
