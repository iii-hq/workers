import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import * as events from '../../src/turn-orchestrator/events.js';
import * as chainModule from '../../src/turn-orchestrator/hooks/chain.js';
import { FakeSessionManager } from '../_helpers/fakeSessionManager.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import type { TurnStateRecord } from '../../src/turn-orchestrator/state.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import * as agentTriggerModule from '../../src/turn-orchestrator/agent-trigger.js';
import { handleExecute } from '../../src/turn-orchestrator/function-execute/process.js';
import { enterFunctionExecute } from '../../src/turn-orchestrator/function-execute/run.js';
import type { FunctionBatchWork } from '../../src/turn-orchestrator/function-execute/types.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

afterEach(() => {
  vi.restoreAllMocks();
});

function mockFinalizePersistence(): void {
  installMockTurnStore({
    appendMessages: vi.fn(async () => {}),
  });
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

/** Wrap a target function id in the agent_trigger envelope (production shape). */
function agentTriggerCall(
  id: string,
  functionId: string,
  payload: unknown = {},
): { id: string; function_id: string; arguments: unknown } {
  return { id, function_id: 'agent_trigger', arguments: { function: functionId, payload } };
}

describe('handleExecute new flow', () => {
  it('runs the prepared batch from work', async () => {
    vi.spyOn(agentTriggerModule, 'triggerWithHook').mockResolvedValueOnce({
      kind: 'result',
      result: {
        content: [{ type: 'text' as const, text: 'ok' }],
        details: {},
        terminate: false,
      },
    });
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    enterFunctionExecute(
      rec,
      makeAssistant([agentTriggerCall('fc-1', 'shell::run', { command: 'ls' })]),
    );
    rec.state = 'function_execute';

    mockFinalizePersistence();
    await handleExecute(iii, rec);
    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    const turnEnd = emitSpy.mock.calls.find((call) => call[2]?.type === 'turn_end')?.[2] as
      | { function_results: Array<{ function_call_id: string }> }
      | undefined;
    expect(turnEnd?.function_results).toHaveLength(1);
    expect(turnEnd?.function_results[0]?.function_call_id).toBe('fc-1');
  });

  it('finishes the session when every function result terminates', async () => {
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    seedFunctionExecute(rec, {
      prepared: [{ route: 'trigger', call: fc }],
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

    expect(rec.state).toBe('finishing');
  });

  it('does not re-emit function_execution_start for already-executed calls on re-entry', async () => {
    const emitted: Array<{ type: string; function_call_id?: string }> = [];
    vi.spyOn(events, 'emit').mockImplementation(async (_iii, _sid, ev: never) => {
      emitted.push(ev as { type: string; function_call_id?: string });
    });
    vi.spyOn(agentTriggerModule, 'triggerWithHook').mockResolvedValueOnce({
      kind: 'result',
      result: { content: [{ type: 'text' as const, text: 'ok' }], details: {}, terminate: false },
    });
    const iii = { trigger: vi.fn().mockResolvedValue(null) } as unknown as ISdk;
    const rec: TurnStateRecord = newRecord('s1');
    const fc1 = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const fc2 = { id: 'fc-2', function_id: 'shell::run', arguments: {} };
    seedFunctionExecute(rec, {
      prepared: [
        { route: 'trigger', call: fc1 },
        { route: 'trigger', call: fc2 },
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

  it('skips the hook chain on pre_approved entries and uses triggerFunctionCall', async () => {
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
    const consultSpy = vi.spyOn(chainModule, 'consultPreTrigger');
    mockFinalizePersistence();

    await handleExecute(iii, rec);

    expect(consultSpy).not.toHaveBeenCalled();
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
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await expect(handleExecute(iii, rec)).resolves.toBeUndefined();

    expect(rec.state).toBe('assistant_streaming');
    const turnEnd = emitSpy.mock.calls.find((call) => call[2]?.type === 'turn_end')?.[2] as
      | { function_results: Array<{ is_error: boolean; details: Record<string, unknown> }> }
      | undefined;
    expect(turnEnd?.function_results).toHaveLength(1);
    expect(turnEnd?.function_results[0]?.is_error).toBe(true);
    const details = turnEnd?.function_results[0]?.details as Record<string, unknown>;
    expect(details?.status).toBe('denied');
    expect(details?.denied_by).toBe('gate_unavailable');
    expect(details?.function_id).toBe('shell::fs::write');
    expect(String(details?.reason)).toContain('S210');
  });

  it('emits denial result without triggering when route is synthetic', async () => {
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
    expect(rec.state).toBe('assistant_streaming');
  });

  it('replays persisted executed calls without re-triggering (re-entry with pre-populated work.executed)', async () => {
    const triggerWithHookSpy = vi.spyOn(agentTriggerModule, 'triggerWithHook');
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
      prepared: [{ route: 'trigger', call: fc }],
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

    expect(triggerWithHookSpy).not.toHaveBeenCalled();
    expect(rec.state).toBe('assistant_streaming');
  });

  it('resumes to assistant_streaming after a successful hook trigger', async () => {
    vi.spyOn(agentTriggerModule, 'triggerWithHook').mockResolvedValueOnce({
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

    expect(rec.state).toBe('assistant_streaming');
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
        prepared: [{ route: 'trigger', call: fc }],
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
      appendMessages: vi.fn(async () => {}),
    });
    const emitSpy = vi.spyOn(events, 'emit').mockResolvedValue(undefined);

    await handleExecute(iii, rec);

    expect(rec.state).toBe('assistant_streaming');
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

    // Route session::* into the in-memory fake so the real store's
    // fr-<call_id> entry-id idempotency is what dedupes the re-entry.
    const sessions = new FakeSessionManager();
    const iii = {
      trigger: vi.fn(
        async ({ function_id, payload }: { function_id: string; payload: unknown }) =>
          (await sessions.handle(function_id, payload)) ?? null,
      ),
    } as unknown as ISdk;
    const rec = newRecord('s1');
    rec.state = 'function_execute';
    enterFunctionExecute(
      rec,
      makeAssistant([agentTriggerCall('toolu_01', 'shell::run', { command: 'ls' })]),
    );

    vi.spyOn(events, 'emit').mockResolvedValue(undefined);
    vi.spyOn(agentTriggerModule, 'triggerWithHook').mockResolvedValue({
      kind: 'result',
      result: existingResult,
    });

    await handleExecute(iii, rec);

    const asst = makeAssistant([agentTriggerCall('toolu_01', 'shell::run', { command: 'ls' })]);
    seedFunctionExecute(
      rec,
      {
        prepared: [{ route: 'trigger', call: fc }],
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

    const fnResults = sessions
      .messageEntries('s1')
      .filter((e) => e.message?.role === 'function_result');
    expect(fnResults).toHaveLength(1);
    expect(
      (fnResults[0]?.message as { function_call_id?: string } | undefined)?.function_call_id,
    ).toBe('toolu_01');
  });
});
