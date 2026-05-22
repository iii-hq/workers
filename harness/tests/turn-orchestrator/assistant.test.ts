import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';
import { TOOL_NAME } from '../../src/turn-orchestrator/agent-trigger.js';
import * as persistence from '../../src/turn-orchestrator/persistence.js';
import * as preflightModule from '../../src/turn-orchestrator/preflight.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleFinished } from '../../src/turn-orchestrator/states/assistant-finished.js';
import { handleStreaming } from '../../src/turn-orchestrator/states/assistant-streaming.js';

type TriggerCall = { function_id: string; payload: unknown; timeoutMs?: number };

function fakeIii(overrides?: Partial<ISdk>): { iii: ISdk; calls: TriggerCall[] } {
  const calls: TriggerCall[] = [];
  const iii = {
    trigger: async <T, R>(req: {
      function_id: string;
      payload: T;
      timeoutMs?: number;
    }): Promise<R> => {
      calls.push({
        function_id: req.function_id,
        payload: req.payload,
        timeoutMs: req.timeoutMs,
      });
      return null as R;
    },
    ...overrides,
  } as unknown as ISdk;
  return { iii, calls };
}

function assistant(overrides: Partial<AssistantMessage> = {}): AssistantMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text: 'hello' }],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: 'gpt-4o',
    provider: 'openai',
    timestamp: 1,
    ...overrides,
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('handleStreaming turn start', () => {
  it('starts a normal assistant turn without approval::consume resurrection', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii, calls } = fakeIii({
      createChannel: async () => {
        throw new Error('channel unavailable');
      },
    });
    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: 'openai',
      model: 'gpt-4o',
    });
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'loadFunctionSchemas').mockResolvedValue([]);
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.turn_count).toBe(1);
    expect(rec.turn_end_emitted).toBe(false);
    expect(calls.some((c) => c.function_id === 'approval::consume')).toBe(false);
    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
  });

  it('exhausts max_turns and transitions to tearing_down', async () => {
    const rec: TurnStateRecord = {
      ...newRecord('s1', 2),
      state: 'assistant_streaming',
      turn_count: 2,
    };
    const { iii, calls } = fakeIii();
    const saveSpy = vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('tearing_down');
    expect(rec.turn_end_emitted).toBe(true);
    expect(rec.last_assistant?.content[0]).toEqual({
      type: 'text',
      text: 'loop stopped: max_turns (2) reached',
    });
    expect(saveSpy).toHaveBeenCalledOnce();
    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
  });
});

describe('handleStreaming', () => {
  it('transitions to assistant_finished with synthetic error when createChannel fails', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii } = fakeIii({
      createChannel: async () => {
        throw new Error('channel unavailable');
      },
    });
    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: 'openai',
      model: 'gpt-4o',
    });
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'loadFunctionSchemas').mockResolvedValue([]);
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('assistant_finished');
    expect(rec.last_assistant?.stop_reason).toBe('error');
    expect(rec.last_assistant?.error_message).toContain('create_channel failed');
  });

  it('captures provider done frame and transitions to assistant_finished', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const finalMsg = assistant({ content: [{ type: 'text', text: 'done reply' }] });
    let deliver: ((msg: string) => void) | null = null;

    const { iii } = fakeIii({
      createChannel: async () => ({
        writerRef: {},
        reader: {
          onMessage: (cb: (msg: string) => void) => {
            deliver = cb;
          },
          stream: {
            resume: () => {
              deliver?.(
                JSON.stringify({
                  type: 'done',
                  message: finalMsg,
                }),
              );
            },
          },
        },
      }),
    });

    vi.spyOn(persistence, 'loadRunRequest').mockResolvedValue({
      provider: 'openai',
      model: 'gpt-4o',
    });
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'loadFunctionSchemas').mockResolvedValue([]);
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('assistant_finished');
    expect(rec.last_assistant).toEqual(finalMsg);
  });
});

describe('handleFinished', () => {
  it('throws when last_assistant is missing', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_finished' };
    const { iii } = fakeIii();

    await expect(handleFinished(iii, rec)).rejects.toThrow(
      'assistant_finished without last_assistant',
    );
  });

  it('routes error assistant to tearing_down without persisting transcript', async () => {
    const rec: TurnStateRecord = {
      ...newRecord('s1'),
      state: 'assistant_finished',
      last_assistant: assistant({ stop_reason: 'error', error_message: 'auth failed' }),
    };
    const { iii } = fakeIii();
    const saveSpy = vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);

    await handleFinished(iii, rec);

    expect(rec.state).toBe('tearing_down');
    expect(rec.turn_end_emitted).toBe(true);
    expect(saveSpy).not.toHaveBeenCalled();
  });

  it('routes text-only assistant to steering_check and persists message', async () => {
    const rec: TurnStateRecord = {
      ...newRecord('s1'),
      state: 'assistant_finished',
      last_assistant: assistant(),
    };
    const { iii } = fakeIii();
    const saveSpy = vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);

    await handleFinished(iii, rec);

    expect(rec.state).toBe('steering_check');
    expect(rec.pending_function_calls).toEqual([]);
    expect(saveSpy).toHaveBeenCalledOnce();
  });

  it('prepares function calls and transitions to function_execute', async () => {
    const rec: TurnStateRecord = {
      ...newRecord('s1'),
      state: 'assistant_finished',
      last_assistant: assistant({
        content: [
          {
            type: 'function_call',
            id: 'fc-1',
            function_id: 'shell::run',
            arguments: { command: 'ls' },
          },
        ],
      }),
    };
    const { iii } = fakeIii();
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    const saveExecutedSpy = vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);
    const savePreparedSpy = vi.spyOn(persistence, 'savePreparedCalls').mockResolvedValue(undefined);

    await handleFinished(iii, rec);

    expect(rec.state).toBe('function_execute');
    expect(rec.function_results).toEqual([]);
    expect(rec.pending_function_calls).toEqual([
      { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
    ]);
    expect(saveExecutedSpy).toHaveBeenCalledWith(iii, 's1', []);
    expect(savePreparedSpy).toHaveBeenCalledWith(iii, 's1', [
      {
        function_call: { id: 'fc-1', function_id: 'shell::run', arguments: { command: 'ls' } },
        blocked: null,
      },
    ]);
  });

  it('does NOT duplicate the assistant message when handleFinished re-enters', async () => {
    // Idempotency guard: a durable retry / crash-before-transitionTo can
    // replay handleFinished with the same last_assistant. Re-pushing a
    // tool-call assistant makes Anthropic reject the next request with
    // "each tool_use must have a unique id".
    const rec: TurnStateRecord = {
      ...newRecord('s1'),
      state: 'assistant_finished',
      last_assistant: assistant({
        content: [
          {
            type: 'function_call',
            id: 'toolu_42',
            function_id: 'shell::run',
            arguments: { command: 'pwd' },
          },
        ],
      }),
    };
    const { iii } = fakeIii();
    let storedMessages: unknown[] = [];
    vi.spyOn(persistence, 'loadMessages').mockImplementation(async () => storedMessages as never);
    vi.spyOn(persistence, 'saveMessages').mockImplementation(async (_iii, _sid, msgs) => {
      storedMessages = msgs as never;
    });
    vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'savePreparedCalls').mockResolvedValue(undefined);

    await handleFinished(iii, rec);
    // Re-entry: same record before the transition was durably observed.
    rec.state = 'assistant_finished';
    await handleFinished(iii, rec);

    const asstMsgs = (storedMessages as Array<{ role?: string }>).filter(
      (m) => m.role === 'assistant',
    );
    expect(asstMsgs).toHaveLength(1);
  });

  it('unwraps agent_trigger wrappers when preparing function calls', async () => {
    const rec: TurnStateRecord = {
      ...newRecord('s1'),
      state: 'assistant_finished',
      last_assistant: assistant({
        content: [
          {
            type: 'function_call',
            id: 'fc-wrap',
            function_id: TOOL_NAME,
            arguments: { function: 'shell::run', payload: { command: 'ls' } },
          },
          {
            type: 'function_call',
            id: 'fc-direct',
            function_id: 'shell::echo',
            arguments: { text: 'hi' },
          },
        ],
      }),
    };
    const { iii } = fakeIii();
    vi.spyOn(persistence, 'loadMessages').mockResolvedValue([]);
    vi.spyOn(persistence, 'saveMessages').mockResolvedValue(undefined);
    vi.spyOn(persistence, 'saveExecutedCalls').mockResolvedValue(undefined);
    const savePreparedSpy = vi.spyOn(persistence, 'savePreparedCalls').mockResolvedValue(undefined);

    await handleFinished(iii, rec);

    expect(rec.state).toBe('function_execute');
    const prepared = savePreparedSpy.mock.calls[0]?.[2];
    expect(prepared).toEqual([
      {
        function_call: { id: 'fc-wrap', function_id: 'shell::run', arguments: { command: 'ls' } },
        blocked: null,
      },
      {
        function_call: { id: 'fc-direct', function_id: 'shell::echo', arguments: { text: 'hi' } },
        blocked: null,
      },
    ]);
  });
});
