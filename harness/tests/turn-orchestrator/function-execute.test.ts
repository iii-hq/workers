import { describe, expect, it, vi } from 'vitest';
import {
  missingFunctionResult,
  unwrapAgentTrigger,
} from '../../src/turn-orchestrator/agent-trigger.js';
import {
  enterFunctionExecute,
  finalizeBatch,
  runOneCall,
} from '../../src/turn-orchestrator/function-execute/run.js';
import {
  withRoutingEnvelope,
} from '../../src/turn-orchestrator/function-execute/ports.js';
import type { FunctionExecutePorts } from '../../src/turn-orchestrator/function-execute/ports.js';
import type { ExecutedCall } from '../../src/turn-orchestrator/function-execute/types.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

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

function stubPorts(overrides: Partial<FunctionExecutePorts> = {}): FunctionExecutePorts {
  return {
    emitStart: vi.fn(async () => {}),
    emitEnd: vi.fn(async () => {}),
    emit: vi.fn(async () => {}),
    checkpoint: vi.fn(async () => {}),
    dispatch: vi.fn(async () => ({
      kind: 'result' as const,
      result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
    })),
    triggerPreApproved: vi.fn(async () => ({
      content: [{ type: 'text' as const, text: 'ok' }],
      details: {},
    })),
    loadMessages: vi.fn(async () => []),
    appendMessages: vi.fn(async () => {}),
    emitTurnEnd: vi.fn(async () => {}),
    finishSession: vi.fn(async (rec) => {
      rec.state = 'stopped';
    }),
    ...overrides,
  };
}

function preparedFromAssistant(asst: AssistantMessage) {
  const rec = newRecord('s1');
  enterFunctionExecute(rec, asst);
  return rec.work!.prepared;
}

describe('batch planning from assistant', () => {
  it('unwraps agent_trigger and maps empty function_id to synthetic', () => {
    const batch = preparedFromAssistant(
      makeAssistant([
        {
          id: 'fc-1',
          function_id: 'agent_trigger',
          arguments: { function: 'shell::run', payload: { x: 1 } },
        },
        { id: 'fc-2', function_id: 'agent_trigger', arguments: {} },
      ]),
    );

    expect(batch[0]).toEqual({
      route: 'dispatch',
      call: unwrapAgentTrigger({
        id: 'fc-1',
        function_id: 'agent_trigger',
        arguments: { function: 'shell::run', payload: { x: 1 } },
      }),
    });
    expect(batch[1]).toMatchObject({
      route: 'synthetic',
      result: missingFunctionResult(),
    });
  });

  it('maps non-agent_trigger function_id to synthetic error', () => {
    const batch = preparedFromAssistant(
      makeAssistant([{ id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } }]),
    );
    expect(batch[0]).toMatchObject({
      route: 'synthetic',
      result: missingFunctionResult(),
    });
  });
});

describe('withRoutingEnvelope', () => {
  it('merges routing fields without mutating the original call object', () => {
    const call = { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } };
    const augmented = withRoutingEnvelope(call, 'sess-1');
    expect(augmented.arguments).toMatchObject({
      command: 'ls',
      session_id: 'sess-1',
      function_call_id: 'fc-1',
      function_id: 'shell::run',
    });
    expect(call.arguments).toEqual({ command: 'ls' });
  });
});

describe('runOneCall', () => {
  it('replays end event only when call id is already executed', async () => {
    const ports = stubPorts();
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const executed: Record<string, ExecutedCall> = {
      'fc-1': {
        call: fc,
        result: { content: [{ type: 'text' as const, text: 'cached' }], details: {} },
        is_error: false,
        duration_ms: 10,
      },
    };

    const outcome = await runOneCall(ports, 's1', { route: 'dispatch', call: fc }, executed);

    expect(outcome.kind).toBe('skipped');
    expect(ports.emitStart).not.toHaveBeenCalled();
    expect(ports.emitEnd).toHaveBeenCalledOnce();
    expect(ports.dispatch).not.toHaveBeenCalled();
  });

  it('returns pending without mutating executed map', async () => {
    const ports = stubPorts({
      dispatch: vi.fn(async () => ({ kind: 'pending' as const })),
    });
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const executed: Record<string, ExecutedCall> = {};

    const outcome = await runOneCall(ports, 's1', { route: 'dispatch', call: fc }, executed);

    expect(outcome.kind).toBe('pending');
    expect(executed).toEqual({});
  });
});

describe('finalizeBatch', () => {
  it('routes to stopped when every result terminates', async () => {
    const ports = stubPorts();
    const rec = newRecord('s1');
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    enterFunctionExecute(rec, makeAssistant([fc]));
    rec.state = 'function_execute';

    rec.work = {
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
    };
    await finalizeBatch(ports, rec);

    expect(rec.state).toBe('finishing');
    expect(ports.finishSession).not.toHaveBeenCalled();
  });

  it('appends results in one idempotent batch (no transcript reload for dedup)', async () => {
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const appendMessages = vi.fn(async () => {});
    const loadMessages = vi.fn(async () => []);
    const ports = stubPorts({ loadMessages, appendMessages });
    const rec = newRecord('s1');
    enterFunctionExecute(rec, makeAssistant([fc]));
    rec.state = 'function_execute';

    rec.work = {
      prepared: [{ route: 'dispatch', call: fc }],
      executed: {
        'fc-1': {
          call: fc,
          result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
          is_error: false,
          duration_ms: 1,
        },
      },
    };
    await finalizeBatch(ports, rec);

    expect(loadMessages).not.toHaveBeenCalled();
    expect(appendMessages).toHaveBeenCalledTimes(1);
    expect(appendMessages).toHaveBeenCalledWith(
      's1',
      [expect.objectContaining({ role: 'function_result', function_call_id: 'fc-1' })],
      expect.objectContaining({ origin: { turn: rec.turn_count } }),
    );
    expect(rec.state).toBe('assistant_streaming');
  });

  it('resumes to assistant_streaming with cleared results when a result continues', async () => {
    const ports = stubPorts();
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const rec = newRecord('s1');
    enterFunctionExecute(rec, makeAssistant([fc]));
    rec.state = 'function_execute';
    rec.work = {
      prepared: [{ route: 'dispatch', call: fc }],
      executed: {
        'fc-1': {
          call: fc,
          result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
          is_error: false,
          duration_ms: 1,
        },
      },
    };

    await finalizeBatch(ports, rec);

    expect(rec.state).toBe('assistant_streaming');
    expect(rec.function_results).toEqual([]);
    expect(rec.turn_end_emitted).toBe(true);
    expect(ports.finishSession).not.toHaveBeenCalled();
  });

  it('ends the loop inline at the max_turns cap instead of resuming', async () => {
    const appendMessages = vi.fn(async () => {});
    const emit = vi.fn(async () => {});
    const ports = stubPorts({ appendMessages, emit });
    const fc = { id: 'fc-1', function_id: 'shell::run', arguments: {} };
    const rec = { ...newRecord('s1', 2), turn_count: 2 };
    enterFunctionExecute(rec, makeAssistant([fc]));
    rec.state = 'function_execute';
    rec.work = {
      prepared: [{ route: 'dispatch', call: fc }],
      executed: {
        'fc-1': {
          call: fc,
          result: { content: [{ type: 'text' as const, text: 'ok' }], details: {} },
          is_error: false,
          duration_ms: 1,
        },
      },
    };

    await finalizeBatch(ports, rec);

    expect(rec.state).toBe('finishing');
    expect(ports.finishSession).not.toHaveBeenCalled();
    expect(appendMessages).toHaveBeenCalledTimes(2);
    const appended = appendMessages.mock.calls[1]?.[1] as Array<{ content: unknown[] }>;
    expect(JSON.stringify(appended)).toContain('max_turns (2) reached');
    expect(
      emit.mock.calls.some((call) => (call[1] as { type: string })?.type === 'message_complete'),
    ).toBe(true);
  });
});
