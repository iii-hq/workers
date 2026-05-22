import { describe, expect, it } from 'vitest';
import {
  PLACEHOLDER_USER_MESSAGE,
  toOpenaiMessages,
} from '../../src/provider-llamacpp/wire-messages.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

describe('toOpenaiMessages (llamacpp)', () => {
  it('passes through plain user → assistant turns verbatim', () => {
    const out = toOpenaiMessages(
      [
        {
          role: 'user',
          content: [{ type: 'text', text: 'hello' }],
          timestamp: 0,
        },
      ],
      'be helpful',
    ) as Array<Record<string, unknown>>;
    expect(out[0]).toEqual({ role: 'system', content: 'be helpful' });
    expect(out[1]).toEqual({ role: 'user', content: 'hello' });
  });

  it('round-trips reasoning_content on assistant tool-call messages', () => {
    const messages: AgentMessage[] = [
      {
        role: 'user',
        content: [{ type: 'text', text: 'q' }],
        timestamp: 0,
      },
      {
        role: 'assistant',
        content: [
          { type: 'thinking', text: 'thinking step' },
          {
            type: 'function_call',
            id: 'call_01',
            function_id: 'fs::ls',
            arguments: { path: '.' },
          },
        ],
        stop_reason: 'function_call',
        model: 'm',
        provider: 'llamacpp',
        timestamp: 0,
      },
    ];
    const out = toOpenaiMessages(messages, '') as Array<Record<string, unknown>>;
    const asst = out.find((m) => m.role === 'assistant') as Record<string, unknown>;
    expect(asst).toBeDefined();
    expect(asst.reasoning_content).toBe('thinking step');
    expect(Array.isArray(asst.tool_calls)).toBe(true);
  });

  it('emits tool messages with content + tool_call_id + is_error', () => {
    const messages: AgentMessage[] = [
      {
        role: 'function_result',
        function_call_id: 'tc1',
        function_id: 'fs::read',
        content: [{ type: 'text', text: '...' }],
        details: {},
        is_error: true,
        timestamp: 0,
      },
    ];
    const out = toOpenaiMessages(messages, '') as Array<Record<string, unknown>>;
    const tool = out.find((m) => m.role === 'tool') as Record<string, unknown>;
    expect(tool.tool_call_id).toBe('tc1');
    expect(tool.is_error).toBe(true);
  });

  it('injects a placeholder user message when no user-role message is present', () => {
    const messages: AgentMessage[] = [
      {
        role: 'function_result',
        function_call_id: 'tc1',
        function_id: 'fs::read',
        content: [{ type: 'text', text: 'orphan' }],
        details: {},
        is_error: false,
        timestamp: 0,
      },
    ];
    const out = toOpenaiMessages(messages, '') as Array<Record<string, unknown>>;
    const userRow = out.find((m) => m.role === 'user');
    expect(userRow).toBeDefined();
    expect((userRow as { content: string }).content).toBe(PLACEHOLDER_USER_MESSAGE);
  });

  it('dedupes function_results with the same tool_call_id (latest wins)', () => {
    const mk = (id: string, text: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'fs::read',
      content: [{ type: 'text', text }],
      details: {},
      is_error: false,
      timestamp: 0,
    });
    const out = toOpenaiMessages([mk('a', 'first'), mk('a', 'second')], '') as Array<
      Record<string, unknown>
    >;
    const tools = out.filter((m) => m.role === 'tool');
    expect(tools).toHaveLength(1);
    expect(tools[0]?.content).toBe('second');
  });

  it('preserves order of distinct tool_call_ids while deduping repeats', () => {
    const mk = (id: string, text: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'fs::read',
      content: [{ type: 'text', text }],
      details: {},
      is_error: false,
      timestamp: 0,
    });
    const out = toOpenaiMessages(
      [mk('a', 'A1'), mk('b', 'B1'), mk('a', 'A2'), mk('c', 'C1'), mk('b', 'B2')],
      '',
    ) as Array<Record<string, unknown>>;
    const tools = out.filter((m) => m.role === 'tool') as Array<{
      tool_call_id: string;
      content: string;
    }>;
    expect(tools.map((t) => t.tool_call_id)).toEqual(['a', 'b', 'c']);
    expect(tools[0]?.content).toBe('A2');
    expect(tools[1]?.content).toBe('B2');
    expect(tools[2]?.content).toBe('C1');
  });
});
