import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as events from '../../src/turn-orchestrator/events.js';
import * as hookModule from '../../src/turn-orchestrator/hook.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import * as agentTriggerModule from '../../src/turn-orchestrator/agent-trigger.js';
import * as approvalResumeModule from '../../src/turn-orchestrator/approval-resume.js';
import { parseApprovalDecision } from '../../src/turn-orchestrator/states/function-awaiting-approval.js';
import { handleExecute } from '../../src/turn-orchestrator/states/function-execute.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

afterEach(() => {
  vi.restoreAllMocks();
});

function mockFinalizePersistence(): void {
  vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
  vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
  vi.spyOn(hookModule, 'publishAfter').mockResolvedValue(undefined);
}

/** Build a minimal AssistantMessage with the given function_call content blocks. */
function makeAssistant(
  calls: Array<{ id: string; function_id: string; arguments?: unknown }>,
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

describe('handleExecute new flow', () => {
  it('builds work.batch from last_assistant when work is absent', async () => {
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
    rec.state = 'function_execute';
    rec.last_assistant = makeAssistant([
      { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
    ]);

    mockFinalizePersistence();
    await handleExecute(iii, rec);

    // work should be cleared after finalize
    expect(rec.work).toBeUndefined();
    expect(rec.state).toBe('steering_check');
    expect(rec.function_results).toHaveLength(1);
    expect(rec.function_results[0]?.function_call_id).toBe('fc-1');
  });

  it('pushes the call onto awaiting_approval and transitions to function_awaiting_approval on pending', async () => {
    const dispatchSpy = vi.spyOn(agentTriggerModule, 'dispatchWithHook');
    dispatchSpy.mockResolvedValueOnce({ kind: 'pending' });
    const registerResumeSpy = vi
      .spyOn(approvalResumeModule, 'registerApprovalResume')
      .mockReturnValue({ unregister: vi.fn() } as never);

    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    rec.state = 'function_execute';
    rec.last_assistant = makeAssistant([
      { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
    ]);

    await handleExecute(iii, rec);

    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
    expect(rec.awaiting_approval?.[0]?.function_call_id).toBe('fc-1');
    expect(registerResumeSpy).toHaveBeenCalledWith(iii, 's1', 'fc-1');
    // work.batch should still be populated (re-entry will continue from it)
    expect(rec.work?.batch).toHaveLength(1);
  });

  it('skips consultBefore on pre_approved entries and uses triggerFunctionCall', async () => {
    const triggerSpy = vi.fn().mockResolvedValue({ ok: true });
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    rec.state = 'function_execute';
    // Supply via rec.work (simulates re-entry after approval was granted)
    rec.work = {
      batch: [
        {
          function_call: {
            id: 'fc-1',
            function_id: 'shell::run',
            arguments: { command: 'ls' },
          },
          blocked: null,
          pre_approved: true,
        },
      ],
      results: [],
    };
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
    rec.state = 'function_execute';
    rec.work = {
      batch: [
        {
          function_call: {
            id: 'fc-1',
            function_id: 'shell::fs::write',
            arguments: { content: 'Tue May 19 08:17:10 -03 2026\n' },
          },
          blocked: null,
          pre_approved: true,
        },
      ],
      results: [],
    };
    mockFinalizePersistence();

    await expect(handleExecute(iii, rec)).resolves.toBeUndefined();

    expect(rec.state).toBe('steering_check');
    // The result should be an error with denied details
    expect(rec.function_results).toHaveLength(1);
    expect(rec.function_results[0]?.is_error).toBe(true);
    const details = rec.function_results[0]?.details as Record<string, unknown>;
    expect(details?.status).toBe('denied');
    expect(details?.denied_by).toBe('gate_unavailable');
    expect(details?.function_id).toBe('shell::fs::write');
    expect(String(details?.reason)).toContain('S210');
  });

  it('emits denial result without dispatching when blocked is set', async () => {
    const triggerSpy = vi.fn().mockResolvedValue(null);
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    rec.state = 'function_execute';

    const denial = {
      content: [{ type: 'text' as const, text: 'denied' }],
      details: { approval_denied: true, decision: 'deny' as const },
      terminate: false,
    };
    rec.work = {
      batch: [
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          blocked: denial,
          pre_approved: false,
        },
      ],
      results: [],
    };
    mockFinalizePersistence();
    await handleExecute(iii, rec);

    const shellCalls = triggerSpy.mock.calls.filter(
      (call) => (call[0] as { function_id: string }).function_id === 'shell::run',
    );
    expect(shellCalls).toHaveLength(0);
    expect(rec.state).toBe('steering_check');
  });

  it('replays persisted executed calls without re-dispatching (re-entry with pre-populated work.results)', async () => {
    const dispatchSpy = vi.spyOn(agentTriggerModule, 'dispatchWithHook');
    const triggerSpy = vi.fn().mockResolvedValue(null);
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';

    const existingResult = {
      content: [{ type: 'text' as const, text: 'cached' }],
      details: {},
      terminate: false,
    };
    // Pre-populate rec.work with batch + already-executed result
    rec.work = {
      batch: [
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          blocked: null,
        },
      ],
      results: [
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          result: existingResult,
          is_error: false,
          duration_ms: 42,
        },
      ],
    };
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
    rec.last_assistant = makeAssistant([
      { id: 'fc-1', function_id: 'shell::run', arguments: {} },
    ]);

    mockFinalizePersistence();
    await handleExecute(iii, rec);

    expect(rec.state).toBe('steering_check');
  });

  it('transitions to steering_check when last_assistant is missing after execute (with pre-populated work)', async () => {
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    rec.last_assistant = null;

    // Supply pre-populated work so ensureWork doesn't throw on null last_assistant
    rec.work = {
      batch: [
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          blocked: null,
        },
      ],
      results: [
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          result: {
            content: [{ type: 'text' as const, text: 'ok' }],
            details: {},
            terminate: false,
          },
          is_error: false,
          duration_ms: 1,
        },
      ],
    };
    vi.spyOn(hookModule, 'publishAfter').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleExecute(iii, rec);

    expect(rec.state).toBe('steering_check');
    expect(rec.pending_function_calls).toEqual([]);
    expect(rec.function_results).toHaveLength(1);
    // No turn_end emitted when last_assistant is null
    expect(emitSpy.mock.calls.some((call) => call[2]?.type === 'turn_end')).toBe(false);
  });

  it('emits turn lifecycle and sets turn_end_emitted when last_assistant is present', async () => {
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    rec.last_assistant = {
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

    // Supply pre-populated work (last_assistant has no function_call blocks here)
    rec.work = {
      batch: [],
      results: [
        {
          function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
          result: {
            content: [{ type: 'text' as const, text: 'ok' }],
            details: {},
            terminate: false,
          },
          is_error: false,
          duration_ms: 1,
        },
      ],
    };
    vi.spyOn(hookModule, 'publishAfter').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleExecute(iii, rec);

    expect(rec.state).toBe('steering_check');
    expect(rec.turn_end_emitted).toBe(true);
    expect(emitSpy.mock.calls.some((call) => call[2]?.type === 'turn_end')).toBe(true);
  });

  it('does NOT duplicate function_results in flat-state when handleExecute re-enters', async () => {
    // Idempotency guard: a durable retry / step-fanout race can replay the
    // finalize path with the same work. Re-pushing the same function_result
    // blocks makes Anthropic reject with "each tool_use must have a single
    // result. Found multiple tool_result blocks with id".
    const existingResult = {
      content: [{ type: 'text' as const, text: 'ok' }],
      details: {},
      terminate: false,
    };
    const fc = { id: 'toolu_01', function_id: 'shell::run', arguments: { command: 'ls' } };

    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    rec.last_assistant = makeAssistant([{ id: 'toolu_01', function_id: 'shell::run', arguments: { command: 'ls' } }]);

    let storedMessages: unknown[] = [];
    vi.spyOn(persistence, 'loadMessages').mockImplementation(async () => storedMessages as never);
    vi.spyOn(persistence, 'saveMessages').mockImplementation(async (_iii, _sid, msgs) => {
      storedMessages = msgs as never;
    });
    vi.spyOn(hookModule, 'publishAfter').mockResolvedValue(undefined);
    vi.spyOn(events, 'emit').mockResolvedValue(undefined);
    vi.spyOn(agentTriggerModule, 'dispatchWithHook').mockResolvedValue({
      kind: 'result',
      result: existingResult,
    });

    await handleExecute(iii, rec);

    // Re-entry: simulate state was reset before durable confirmation
    rec.state = 'function_execute';
    rec.turn_end_emitted = false;
    // work.results already has the executed call after first run cleared rec.work,
    // so we need to re-populate work for re-entry simulation
    rec.work = {
      batch: [{ function_call: fc, blocked: null }],
      results: [{ function_call: fc, result: existingResult, is_error: false, duration_ms: 5 }],
    };

    await handleExecute(iii, rec);

    const fnResults = (
      storedMessages as Array<{ role?: string; function_call_id?: string }>
    ).filter((m) => m.role === 'function_result');
    expect(fnResults).toHaveLength(1);
    expect(fnResults[0]?.function_call_id).toBe('toolu_01');
  });
});
