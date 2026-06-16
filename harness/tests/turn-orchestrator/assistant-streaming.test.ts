import { describe, expect, it, vi } from 'vitest';
import {
  finalizeAssistantTurn,
  prepareStreamContext,
  resolveAssistantMessage,
  routeAssistantTurn,
  turnAssistantEntryId,
} from '../../src/turn-orchestrator/assistant-streaming/run.js';
import {
  parseFunctionSchemas,
  type AssistantStreamingPorts,
} from '../../src/turn-orchestrator/assistant-streaming/ports.js';
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
    loadAssembleWindow: vi.fn(async () => ({ window: [] })),
    assembleContext: vi.fn(async (params) => ({
      system_prompt: params.baseSystemPrompt,
      messages: params.window.map((w) => w.message),
      applied: { pruned: false, pruned_tokens: 0, compacted: false },
    })),
    persistCompaction: vi.fn(async () => {}),
    streamTurn: vi.fn(async () => ({ final: null, error: null })),
    emitMessageComplete: vi.fn(async () => {}),
    appendAssistantPlaceholder: vi.fn(async () => {}),
    updateAssistantContent: vi.fn(async () => {}),
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

describe('turnAssistantEntryId', () => {
  it('derives a deterministic id from the per-send message_id and turn', () => {
    const rec = newRecord('s1', undefined, 'msg-7');
    rec.turn_count = 2;
    expect(turnAssistantEntryId(rec)).toBe('msg-7-t2-assistant');
    // Stable across recomputation (step re-entry).
    expect(turnAssistantEntryId(rec)).toBe('msg-7-t2-assistant');
  });

  it('falls back to a per-run stamp when message_id is absent', () => {
    const rec = newRecord('s1');
    rec.turn_count = 1;
    expect(turnAssistantEntryId(rec)).toBe(`s1-${rec.started_at_ms}-t1-assistant`);
  });
});

describe('prepareStreamContext', () => {
  it("assembles the window into system_prompt + messages, excluding this turn's entry", async () => {
    const window = [
      { entry_id: 'u1', message: { role: 'user' as const, content: [], timestamp: 1 } },
    ];
    const loadAssembleWindow = vi.fn(async () => ({ window, previousSummary: 'PRIOR' }));
    const assembleContext = vi.fn(async () => ({
      system_prompt: 'sys\n\n# Conversation summary\nPRIOR',
      messages: [{ role: 'user' as const, content: [], timestamp: 1 }],
      applied: { pruned: false, pruned_tokens: 0, compacted: false },
    }));
    const persistCompaction = vi.fn(async () => {});
    const ports = stubStreamingPorts({ loadAssembleWindow, assembleContext, persistCompaction });
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';

    const ctx = await prepareStreamContext(ports, rec, 'asst-entry-1');

    expect(loadAssembleWindow).toHaveBeenCalledWith('s1', { excludeEntryIds: ['asst-entry-1'] });
    // The anchored summary is passed back into the assemble request.
    expect(assembleContext.mock.calls[0]?.[0]?.previousSummary).toBe('PRIOR');
    // The returned (merged) system prompt + budgeted messages drive router::chat.
    expect(ctx.system_prompt).toContain('# Conversation summary');
    expect(ctx.messages).toEqual([{ role: 'user', content: [], timestamp: 1 }]);
    expect(ctx.tools[0]?.name).toBe('agent_trigger');
    // No fresh compaction this turn → nothing persisted.
    expect(persistCompaction).not.toHaveBeenCalled();
  });

  it('persists the compaction round trip when assemble compacts', async () => {
    const window = [
      { entry_id: 'u1', message: { role: 'user' as const, content: [], timestamp: 1 } },
      {
        entry_id: 'a1',
        message: {
          role: 'assistant' as const,
          content: [],
          stop_reason: 'end' as const,
          model: 'm',
          provider: 'p',
          timestamp: 2,
        },
      },
    ];
    const applied = {
      pruned: false,
      pruned_tokens: 0,
      compacted: true,
      summary: 'S',
      tail_start_index: 1,
      tokens_before: 100,
    };
    const persistCompaction = vi.fn(async () => {});
    const ports = stubStreamingPorts({
      loadAssembleWindow: vi.fn(async () => ({ window })),
      assembleContext: vi.fn(async () => ({ system_prompt: 'sys', messages: [], applied })),
      persistCompaction,
    });
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';

    await prepareStreamContext(ports, rec, 'asst-entry-1');

    expect(persistCompaction).toHaveBeenCalledWith('s1', applied, window);
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

  it('routes text-only assistants to end_turn', () => {
    expect(routeAssistantTurn(assistant()).kind).toBe('end_turn');
  });
});

describe('finalizeAssistantTurn', () => {
  it('stops after a strict final content update on error assistant', async () => {
    const ports = stubStreamingPorts();
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';
    const asst = assistant({ stop_reason: 'error', error_message: 'auth failed' });

    await finalizeAssistantTurn(ports, rec, asst, 'asst-entry-1');

    expect(rec.state).toBe('finishing');
    expect(rec.turn_end_emitted).toBe(true);
    // The (partial / synthetic-error) content still lands on the entry.
    expect(ports.updateAssistantContent).toHaveBeenCalledWith('s1', 'asst-entry-1', asst.content);
  });

  it('updates the entry and routes to function_execute when calls exist', async () => {
    const ports = stubStreamingPorts();
    const rec = newRecord('s1');
    rec.state = 'assistant_streaming';
    const asst = assistant({
      content: [{ type: 'function_call', id: 'fc-1', function_id: 'shell::run', arguments: {} }],
    });

    await finalizeAssistantTurn(ports, rec, asst, 'asst-entry-1');

    expect(ports.updateAssistantContent).toHaveBeenCalledOnce();
    expect(ports.updateAssistantContent).toHaveBeenCalledWith('s1', 'asst-entry-1', asst.content);
    expect(rec.state).toBe('function_execute');
    expect(rec.work?.prepared).toHaveLength(1);
    expect(rec.function_results).toEqual([]);
  });
});
