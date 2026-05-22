import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as agentTriggerModule from '../../src/turn-orchestrator/agent-trigger.js';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import * as hookModule from '../../src/turn-orchestrator/hook.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import * as approvalResumeModule from '../../src/turn-orchestrator/approval-resume.js';
import { handleExecute } from '../../src/turn-orchestrator/states/functions.js';

const cfg: TurnOrchestratorConfig = {
  sync_default_timeout_ms: 120_000,
  system_default_skills: [],
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe('handleExecute new flow', () => {
  it('pushes the call onto awaiting_approval and transitions to function_awaiting_approval on pending', async () => {
    const dispatchSpy = vi.spyOn(agentTriggerModule, 'dispatchWithHook');
    dispatchSpy.mockResolvedValueOnce({ kind: 'pending' });
    const registerResumeSpy = vi
      .spyOn(approvalResumeModule, 'registerApprovalResume')
      .mockReturnValue({ unregister: vi.fn() } as never);

    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    rec.state = 'function_execute';

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
    vi.spyOn(persistence, 'loadExecutedCalls').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);
    await handleExecute(iii, cfg, rec);

    expect(rec.state).toBe('function_awaiting_approval');
    expect(rec.awaiting_approval).toHaveLength(1);
    expect(rec.awaiting_approval?.[0]?.function_call_id).toBe('fc-1');
    expect(registerResumeSpy).toHaveBeenCalledWith(iii, 's1', 'fc-1');
  });

  it('skips dispatchWithHook on pre_approved entries and calls iii.trigger directly', async () => {
    const triggerSpy = vi.fn().mockResolvedValue({ ok: true });
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    rec.state = 'function_execute';

    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      {
        function_call: {
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: { command: 'ls' },
        },
        blocked: null,
        pre_approved: true,
      },
    ]);
    vi.spyOn(persistence, 'loadExecutedCalls').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);
    const consultBeforeSpy = vi.spyOn(hookModule, 'consultBefore');

    await handleExecute(iii, cfg, rec);

    expect(consultBeforeSpy).not.toHaveBeenCalled();
    const triggerCalls = triggerSpy.mock.calls.map(
      (call) => (call[0] as { function_id: string }).function_id,
    );
    expect(triggerCalls).toContain('shell::run');
  });

  it('synthesizes an error result when a pre_approved trigger rejects (does not throw out of handleExecute)', async () => {
    const triggerSpy = vi.fn(async (req: { function_id: string }) => {
      if (req.function_id === 'shell::fs::write') {
        throw new Error('handler error: {"code":"S210","message":"bad write payload"}');
      }
      return null;
    });
    const iii = { trigger: triggerSpy } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    rec.state = 'function_execute';

    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      {
        function_call: {
          id: 'fc-1',
          function_id: 'shell::fs::write',
          arguments: { content: 'Tue May 19 08:17:10 -03 2026\n' },
        },
        blocked: null,
        pre_approved: true,
      },
    ]);
    vi.spyOn(persistence, 'loadExecutedCalls').mockResolvedValue([]);
    const saveSpy = vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);

    await expect(handleExecute(iii, cfg, rec)).resolves.toBeUndefined();

    expect(rec.state).toBe('function_finalize');
    expect(saveSpy).toHaveBeenCalled();
    const lastSave = saveSpy.mock.calls.at(-1)?.[2] as Array<{
      is_error: boolean;
      result: { details: unknown };
    }>;
    expect(lastSave?.[0]?.is_error).toBe(true);
    const details = lastSave?.[0]?.result.details as Record<string, unknown>;
    expect(details?.error).toBe('trigger_failed');
    expect(details?.function).toBe('shell::fs::write');
    expect(String(details?.message)).toContain('S210');
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
    vi.spyOn(persistence, 'loadPreparedCalls').mockResolvedValue([
      {
        function_call: { id: 'fc-1', function_id: 'shell::run', arguments: {} },
        blocked: denial,
        pre_approved: false,
      },
    ]);
    vi.spyOn(persistence, 'loadExecutedCalls').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);
    await handleExecute(iii, cfg, rec);

    const shellCalls = triggerSpy.mock.calls.filter(
      (call) => (call[0] as { function_id: string }).function_id === 'shell::run',
    );
    expect(shellCalls).toHaveLength(0);
    expect(rec.state).toBe('function_finalize');
  });
});

describe('handleFinalize idempotency', () => {
  // Production failure: Anthropic rejected with "each tool_use must have a
  // single result. Found multiple tool_result blocks with id: toolu_...".
  // Root cause: handleFinalize re-entered with the same executedCalls in
  // state and pushed the same function_result AgentMessages onto flat-state
  // a second time. This test pins the idempotent behavior.

  it('does NOT duplicate function_results when handleFinalize runs twice with the same executedCalls', async () => {
    const { handleFinalize } = await import('../../src/turn-orchestrator/states/functions.js');

    const executedCalls = [
      {
        function_call: {
          id: 'toolu_01',
          function_id: 'shell::run',
          arguments: { command: 'ls' },
        },
        result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
        is_error: false,
        duration_ms: 5,
      },
      {
        function_call: {
          id: 'toolu_02',
          function_id: 'fs::read',
          arguments: { path: '/etc/hosts' },
        },
        result: { content: [{ type: 'text' as const, text: '127.0.0.1' }], details: {} },
        is_error: false,
        duration_ms: 3,
      },
    ];

    // executedCalls is still present across both calls — the production
    // failure mode where state wasn't cleared between handleFinalize entries.
    vi.spyOn(persistence, 'loadExecutedCalls').mockResolvedValue(executedCalls);
    vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);

    let storedMessages: unknown[] = [];
    vi.spyOn(persistence, 'loadMessages').mockImplementation(async () => storedMessages as never);
    vi.spyOn(persistence, 'saveMessages').mockImplementation(async (_iii, _sid, msgs) => {
      storedMessages = msgs as never;
    });
    vi.spyOn(hookModule, 'publishAfter').mockResolvedValue(null as never);

    const iii = {
      trigger: vi.fn().mockResolvedValue({ old_value: 0 }),
    } as unknown as ISdk;

    const rec: TurnStateRecord = newRecord('sess-finalize-idem');
    rec.state = 'function_finalize';
    rec.last_assistant = {
      role: 'assistant',
      content: [],
      stop_reason: 'function_call',
      error_message: null,
      error_kind: null,
      usage: { input: 1, output: 1, cache_read: 0, cache_write: 0 },
      model: 'claude',
      provider: 'anthropic',
      timestamp: 0,
    };

    await handleFinalize(iii, rec);
    // Critical: simulate re-entry — handleFinalize fires again with the
    // SAME executedCalls (the production failure mode).
    rec.state = 'function_finalize';
    rec.turn_end_emitted = false;
    await handleFinalize(iii, rec);

    const fnResults = (storedMessages as Array<{ role?: string; function_call_id?: string }>)
      .filter((m) => m.role === 'function_result');
    expect(fnResults).toHaveLength(2);
    expect(fnResults.map((m) => m.function_call_id).sort()).toEqual(['toolu_01', 'toolu_02']);
  });

  it('clears executedCalls at the end so a future re-entry produces zero new results', async () => {
    const { handleFinalize } = await import('../../src/turn-orchestrator/states/functions.js');
    vi.spyOn(persistence, 'loadExecutedCalls').mockResolvedValue([
      {
        function_call: { id: 'toolu_x', function_id: 'f', arguments: {} },
        result: { content: [], details: {} },
        is_error: false,
        duration_ms: 1,
      },
    ]);
    const saveExecutedSpy = vi
      .spyOn(persistence, 'saveExecutedCalls')
      .mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(hookModule, 'publishAfter').mockResolvedValue(null as never);

    const iii = {
      trigger: vi.fn().mockResolvedValue({ old_value: 0 }),
    } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('sess-clear');
    rec.state = 'function_finalize';
    rec.last_assistant = {
      role: 'assistant',
      content: [],
      stop_reason: 'function_call',
      error_message: null,
      error_kind: null,
      usage: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
      model: 'claude',
      provider: 'anthropic',
      timestamp: 0,
    };

    await handleFinalize(iii, rec);

    // saveExecutedCalls(..., []) must have been called at the END.
    const clearCalls = saveExecutedSpy.mock.calls.filter(
      ([, , calls]) => Array.isArray(calls) && (calls as unknown[]).length === 0,
    );
    expect(clearCalls.length).toBeGreaterThan(0);
  });
});

describe('handleFinished idempotency', () => {
  // Sibling hazard to handleFinalize: re-entry pushes the same assistant
  // message twice. If that assistant has tool_calls, Anthropic rejects
  // with "each tool_use must have a unique id".

  it('does NOT duplicate the assistant message when handleFinished runs twice', async () => {
    const { handleFinished } = await import('../../src/turn-orchestrator/states/assistant.js');

    const assistantMsg = {
      role: 'assistant' as const,
      content: [
        {
          type: 'function_call' as const,
          id: 'toolu_42',
          function_id: 'shell::run',
          arguments: { command: 'pwd' },
        },
      ],
      stop_reason: 'function_call' as const,
      error_message: null,
      error_kind: null,
      usage: { input: 10, output: 5, cache_read: 0, cache_write: 0 },
      model: 'claude',
      provider: 'anthropic',
      timestamp: 1700000000,
    };

    let storedMessages: unknown[] = [];
    vi.spyOn(persistence, 'loadMessages').mockImplementation(async () => storedMessages as never);
    vi.spyOn(persistence, 'saveMessages').mockImplementation(async (_iii, _sid, msgs) => {
      storedMessages = msgs as never;
    });

    const iii = {
      trigger: vi.fn().mockResolvedValue({ old_value: 0 }),
    } as unknown as ISdk;

    const rec: TurnStateRecord = newRecord('sess-finished-idem');
    rec.state = 'assistant_finished';
    rec.last_assistant = assistantMsg;

    await handleFinished(iii, rec);
    rec.state = 'assistant_finished';
    rec.turn_end_emitted = false;
    await handleFinished(iii, rec);

    const asstMsgs = (storedMessages as Array<{ role?: string }>).filter(
      (m) => m.role === 'assistant',
    );
    expect(asstMsgs).toHaveLength(1);
  });
});
