import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as agentCallModule from '../../src/turn-orchestrator/agent-call.js';
import type { TurnOrchestratorConfig } from '../../src/turn-orchestrator/config.js';
import * as hookModule from '../../src/turn-orchestrator/hook.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import { handleExecute } from '../../src/turn-orchestrator/states/functions.js';

const cfg: TurnOrchestratorConfig = {
  policy_function_id: 'policy::check_permissions',
  sync_default_timeout_ms: 120_000,
  system_default_skills: [],
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe('handleExecute new flow', () => {
  it('pushes the call onto awaiting_approval and transitions to function_awaiting_approval on pending', async () => {
    const dispatchSpy = vi.spyOn(agentCallModule, 'dispatchWithHook');
    dispatchSpy.mockResolvedValueOnce({ kind: 'pending' });

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
