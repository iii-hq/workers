import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';
import { installMockTurnStore } from './_helpers/mockTurnStore.js';
import { FakeSessionManager } from '../_helpers/fakeSessionManager.js';
import * as preflightModule from '../../src/turn-orchestrator/preflight.js';
import { type TurnStateRecord, newRecord } from '../../src/turn-orchestrator/state.js';
import { handleFinishing } from '../../src/turn-orchestrator/finishing.js';
import { handleStreaming } from '../../src/turn-orchestrator/assistant-streaming/process.js';

type TriggerCall = { function_id: string; payload: unknown; timeoutMs?: number };

function fakeIii(overrides?: Partial<ISdk>): {
  iii: ISdk;
  calls: TriggerCall[];
  sessions: FakeSessionManager;
} {
  const calls: TriggerCall[] = [];
  const sessions = new FakeSessionManager();
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
      const handled = await sessions.handle(req.function_id, req.payload);
      if (handled !== undefined) return handled as R;
      return null as R;
    },
    ...overrides,
  } as unknown as ISdk;
  return { iii, calls, sessions };
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
function fakeIiiWithDone(finalMsg: AssistantMessage): {
  iii: ISdk;
  calls: TriggerCall[];
  sessions: FakeSessionManager;
} {
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
    const { iii, calls, sessions } = fakeIiiWithDone(finalMsg);

    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(calls.some((c) => c.function_id === 'stream::set')).toBe(true);
    const asstEntries = sessions.messageEntries('s1');
    expect(asstEntries).toHaveLength(1);
    expect(asstEntries[0]?.message).toMatchObject({
      role: 'assistant',
      content: finalMsg.content,
    });
    expect(asstEntries[0]?.revision).toBeGreaterThanOrEqual(1);
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

  it('stops on an error assistant, keeping the error content on the session entry', async () => {
    const finalMsg = assistant({ stop_reason: 'error', error_message: 'auth failed' });
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii, sessions } = fakeIiiWithDone(finalMsg);

    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);

    expect(rec.state).toBe('finishing');
    expect(rec.turn_end_emitted).toBe(true);
    const entries = sessions.messageEntries('s1');
    expect(entries).toHaveLength(1);
    expect(entries[0]?.message?.content).toEqual(finalMsg.content);

    await handleFinishing(iii, rec);
    expect(sessions.session('s1').status).toBe('error');
    expect(sessions.session('s1').status_reason).toBe('auth failed');
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
    const rec: TurnStateRecord = { ...newRecord('s1'), state: 'assistant_streaming' };
    const { iii, sessions } = fakeIiiWithDone(finalMsg);

    mockStreamingStore();
    vi.spyOn(preflightModule, 'runPreflight').mockResolvedValue('ok');

    await handleStreaming(iii, rec);
    expect(sessions.messageEntries('s1')).toHaveLength(1);

    // Re-entry: the persisted record had not advanced past assistant_streaming
    // (crash before checkpoint) — same turn re-runs with the same entry id.
    rec.state = 'assistant_streaming';
    rec.turn_count = 0;
    await handleStreaming(iii, rec);

    const asstMsgs = sessions.messageEntries('s1').filter((e) => e.message?.role === 'assistant');
    expect(asstMsgs).toHaveLength(1);
  });
});
