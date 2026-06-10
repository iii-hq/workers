import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import * as preflightModule from '../../src/turn-orchestrator/preflight.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleStreaming } from '../../src/turn-orchestrator/assistant-streaming/process.js';

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

/** Build a fake iii whose createChannel delivers a single done event synchronously on stream.resume(). */
function fakeIiiWithDone(finalMsg: AssistantMessage): { iii: ISdk; calls: TriggerCall[] } {
  return fakeIii({
    createChannel: async () => {
      let deliver: ((msg: string) => void) | null = null;
      return {
        writerRef: {},
        reader: {
          onMessage: (cb: (msg: string) => void) => {
            deliver = cb;
          },
          stream: {
            resume: () => {
              deliver?.(JSON.stringify({ type: 'done', message: finalMsg }));
            },
          },
        },
      };
    },
  });
}

afterEach(() => {
  vi.restoreAllMocks();
});

function mockStreamingStore(overrides: Parameters<typeof installMockTurnStore>[0] = {}) {
  return installMockTurnStore({
    loadRunRequest: vi.fn(async () => ({
      provider: 'openai',
      model: 'gpt-4o',
      mode: null,
      system_prompt: '',
      function_schemas: [],
    })),
    loadMessages: vi.fn(async () => []),
    ...overrides,
  });
}

describe('handleStreaming turn start', () => {
  it('starts a normal assistant turn without approval::consume resurrection', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii, calls } = fakeIii({
      createChannel: async () => {
        throw new Error('channel unavailable');
      },
    });
    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.turn_count).toBe(1);
    expect(rec.turn_end_emitted).toBe(true);
    expect(calls.some((c) => c.function_id === 'approval::consume')).toBe(false);
    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
  });
});

describe('handleStreaming', () => {
  it('stops with a synthetic error when createChannel fails', async () => {
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii } = fakeIii({
      createChannel: async () => {
        throw new Error('channel unavailable');
      },
    });
    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('finishing');
    expect(rec.last_assistant?.stop_reason).toBe('error');
    expect(rec.last_assistant?.error_message).toContain('create_channel failed');
  });

  it('streaming completion emits message_complete, persists, and routes to function_execute when calls exist', async () => {
    const finalMsg = assistant({
      content: [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: { command: 'ls' },
        },
      ],
    });
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii, calls } = fakeIiiWithDone(finalMsg);

    const store = mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');
    const appendSpy = store.appendMessages;

    await handleStreaming(iii, rec);

    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
    expect(appendSpy).toHaveBeenCalledOnce();
    expect(rec.state).toBe('function_execute');
    expect(rec.last_assistant).toEqual(finalMsg);
    expect(rec.function_results).toEqual([]);
    expect(rec.work?.prepared).toHaveLength(1);
  });

  it('ends the turn inline when the assistant made no calls', async () => {
    const finalMsg = assistant({ content: [{ type: 'text', text: 'done reply' }] });
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii } = fakeIiiWithDone(finalMsg);

    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('finishing');
    expect(rec.turn_end_emitted).toBe(true);
    expect(rec.last_assistant).toEqual(finalMsg);
  });

  it('captures provider done frame and routes correctly (text-only → end of turn)', async () => {
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

    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('finishing');
    expect(rec.last_assistant).toEqual(finalMsg);
  });

  it('stops on an error assistant without persisting transcript', async () => {
    const finalMsg = assistant({ stop_reason: 'error', error_message: 'auth failed' });
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii } = fakeIiiWithDone(finalMsg);

    const store = mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');
    const appendSpy = store.appendMessages;

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('finishing');
    expect(rec.turn_end_emitted).toBe(true);
    expect(appendSpy).not.toHaveBeenCalled();
  });

  it('does NOT duplicate the assistant message on re-entry', async () => {
    const finalMsg = assistant({
      content: [
        {
          type: 'function_call',
          id: 'toolu_42',
          function_id: 'shell::run',
          arguments: { command: 'pwd' },
        },
      ],
    });
    let storedMessages: unknown[] = [finalMsg];

    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii } = fakeIiiWithDone(finalMsg);

    mockStreamingStore({
      loadMessages: vi.fn(async () => storedMessages as never),
      appendMessages: vi.fn(async (_sid, msgs) => {
        storedMessages = [...storedMessages, ...msgs];
      }),
    });
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    const asstMsgs = (storedMessages as Array<{ role?: string }>).filter(
      (m) => m.role === 'assistant',
    );
    expect(asstMsgs).toHaveLength(1);
  });
});
