import { describe, expect, it, vi } from 'vitest';
import {
  finalizeAssistantTurn,
  prepareStreamContext,
  resolveAssistantMessage,
  routeAssistantTurn,
  syntheticStreamReason,
} from '../../src/turn-orchestrator/assistant-streaming/run.js';
import {
  parseFunctionSchemas,
  type AssistantStreamingPorts,
} from '../../src/turn-orchestrator/assistant-streaming/ports.js';
import { isDuplicateAssistant } from '../../src/turn-orchestrator/state-runtime/transcript.js';
import { newRecord } from '../../src/turn-orchestrator/state.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';

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

function stubStreamingPorts(
  overrides: Partial<AssistantStreamingPorts> = {},
): AssistantStreamingPorts {
  return {
    loadMessages: vi.fn(async () => []),
    appendMessages: vi.fn(async () => {}),
    checkpoint: vi.fn(async () => {}),
    emitTurnEnd: vi.fn(async () => {}),
    finishSession: vi.fn(async (rec) => {
      rec.state = 'stopped';
    }),
    loadRunRequest: vi.fn(async () => ({
      provider: 'openai',
      model: 'gpt-4o',
      mode: null,
      system_prompt: 'sys',
      function_schemas: [{ name: 'agent_trigger', description: 'd', parameters: {} }],
    })),
    runPreflight: vi.fn(async () => 'ok' as const),
    streamTurn: vi.fn(async () => ({ final: null, error: null })),
    emitMessageUpdate: vi.fn(async () => {}),
    emitMessageComplete: vi.fn(async () => {}),
    persistAssistantIfNew: vi.fn(async () => {}),
    ...overrides,
  };
}

describe('parseFunctionSchemas', () => {
  it('parses valid function schemas via AgentFunctionSchema', () => {
    const tools = parseFunctionSchemas([
      { name: 'agent_trigger', description: 'trigger', parameters: { type: 'object' } },
    ]);
    expect(tools).toHaveLength(1);
    expect(tools[0]?.name).toBe('agent_trigger');
  });
});

describe('prepareStreamContext', () => {
  it('reloads messages when preflight compacts', async () => {
    const loadMessages = vi
      .fn()
      .mockResolvedValueOnce([{ role: 'user', content: [], timestamp: 1 }])
      .mockResolvedValueOnce([{ role: 'user', content: [], timestamp: 2 }]);
    const ports = stubStreamingPorts({
      loadMessages,
      runPreflight: vi.fn(async () => 'compacted'),
    });
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';

    const ctx = await prepareStreamContext(ports, rec);

    expect(loadMessages).toHaveBeenCalledTimes(2);
    expect(ctx.messages).toEqual([{ role: 'user', content: [], timestamp: 2 }]);
    expect(ctx.tools[0]?.name).toBe('agent_trigger');
  });
});

describe('resolveAssistantMessage', () => {
  it('returns the provider final message when present', () => {
    const final = assistant({ content: [{ type: 'text', text: 'done' }] });
    const msg = resolveAssistantMessage(
      { final, error: null, body_streamed: false },
      { provider: 'openai', model: 'gpt-4o' },
    );
    expect(msg).toEqual(final);
    expect(syntheticStreamReason({ final, error: null, body_streamed: false })).toBeNull();
  });

  it('builds a synthetic error when the stream ends without a final', () => {
    const msg = resolveAssistantMessage(
      { final: null, error: 'channel unavailable', body_streamed: false },
      { provider: 'openai', model: 'gpt-4o' },
    );
    expect(msg.stop_reason).toBe('error');
    expect(msg.error_message).toContain('channel unavailable');
  });
});

describe('routeAssistantTurn', () => {
  it('routes error assistants to stopped', () => {
    expect(routeAssistantTurn(assistant({ stop_reason: 'error' })).kind).toBe('stopped');
  });

  it('routes function_call content to function_execute', () => {
    expect(
      routeAssistantTurn(
        assistant({
          content: [
            { type: 'function_call', id: 'fc-1', function_id: 'shell::run', arguments: {} },
          ],
        }),
      ).kind,
    ).toBe('function_execute');
  });

  it('routes text-only assistants to steering_check', () => {
    expect(routeAssistantTurn(assistant()).kind).toBe('steering_check');
  });
});

describe('finalizeAssistantTurn', () => {
  it('stops without persisting on error assistant', async () => {
    const ports = stubStreamingPorts();
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';
    const asst = assistant({ stop_reason: 'error', error_message: 'auth failed' });

    await finalizeAssistantTurn(ports, rec, asst);

    expect(rec.state).toBe('stopped');
    expect(rec.turn_end_emitted).toBe(true);
    expect(ports.persistAssistantIfNew).not.toHaveBeenCalled();
  });

  it('persists and routes to function_execute when calls exist', async () => {
    const ports = stubStreamingPorts();
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';
    const asst = assistant({
      content: [{ type: 'function_call', id: 'fc-1', function_id: 'shell::run', arguments: {} }],
    });

    await finalizeAssistantTurn(ports, rec, asst);

    expect(ports.persistAssistantIfNew).toHaveBeenCalledOnce();
    expect(rec.state).toBe('function_execute');
    expect(rec.work?.prepared).toHaveLength(1);
    expect(rec.function_results).toEqual([]);
  });
});

describe('isDuplicateAssistant', () => {
  it('detects trailing assistant dup for re-entry', () => {
    const asst = assistant({ timestamp: 42, model: 'm', provider: 'p' });
    expect(isDuplicateAssistant([asst], asst)).toBe(true);
    expect(isDuplicateAssistant([], asst)).toBe(false);
  });
});
