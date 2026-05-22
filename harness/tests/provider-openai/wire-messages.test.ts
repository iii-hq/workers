import { describe, expect, it } from 'vitest';
import { toOpenaiMessages } from '../../src/provider-openai/wire-messages.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

describe('toOpenaiMessages', () => {
  it('prepends system message when present', () => {
    const out = toOpenaiMessages([], 'be helpful') as Array<Record<string, unknown>>;
    expect(out[0]).toEqual({ role: 'system', content: 'be helpful' });
  });

  it('encodes assistant tool_calls with stringified arguments', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        { type: 'text', text: 'calling' },
        {
          type: 'function_call',
          id: 'tc1',
          function_id: 'shell::fs::ls',
          arguments: { path: '/tmp' },
        },
      ],
      stop_reason: 'function_call',
      model: 'gpt-5',
      provider: 'openai',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    expect(out[0]?.role).toBe('assistant');
    const tcs = (out[0] as { tool_calls: Array<{ function: { arguments: string } }> }).tool_calls;
    expect(tcs[0]?.function.arguments).toBe('{"path":"/tmp"}');
  });

  it('emits tool messages with content + tool_call_id + is_error', () => {
    const out = toOpenaiMessages(
      [
        {
          role: 'function_result',
          function_call_id: 'tc1',
          function_id: 'read',
          content: [{ type: 'text', text: 'ok' }],
          details: { status: 'denied' },
          is_error: true,
          timestamp: 0,
        },
      ],
      '',
    ) as Array<Record<string, unknown>>;
    expect(out[0]?.role).toBe('tool');
    expect(out[0]?.tool_call_id).toBe('tc1');
    expect(out[0]?.is_error).toBe(true);
    expect(typeof out[0]?.content).toBe('string');
    // denial envelope should be embedded as [PERMISSION_DENIED] prefix
    expect((out[0]?.content as string).startsWith('[PERMISSION_DENIED]')).toBe(true);
  });

  it('joins user text content with newlines', () => {
    const msg: AgentMessage = {
      role: 'user',
      content: [
        { type: 'text', text: 'one' },
        { type: 'text', text: 'two' },
      ],
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    expect(out[0]?.content).toBe('one\ntwo');
  });

  describe('boundary dedup of duplicate tool messages', () => {
    // OpenAI's wire shape emits one `{role:'tool', tool_call_id}` message
    // per function_result (not a bundled content array like Anthropic).
    // Without dedup, orchestrator re-entry would ship two tool messages
    // with the same tool_call_id — some servers reject this, some silently
    // overwrite. Dedup makes behavior deterministic regardless.

    const mkResult = (id: string, text: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'shell::run',
      content: [{ type: 'text', text }],
      details: {},
      is_error: false,
      timestamp: 0,
    });

    it('keeps exactly one tool message per tool_call_id (latest wins)', () => {
      const out = toOpenaiMessages(
        [mkResult('call_01', 'first'), mkResult('call_01', 'second')],
        '',
      ) as Array<Record<string, unknown>>;
      const tools = out.filter((m) => m.role === 'tool');
      expect(tools).toHaveLength(1);
      expect(tools[0]?.tool_call_id).toBe('call_01');
      expect(tools[0]?.content).toBe('second');
    });

    it('preserves order of distinct tool_call_ids', () => {
      const out = toOpenaiMessages(
        [
          mkResult('a', 'A1'),
          mkResult('b', 'B1'),
          mkResult('a', 'A2'),
          mkResult('c', 'C1'),
        ],
        '',
      ) as Array<Record<string, unknown>>;
      const tools = out.filter((m) => m.role === 'tool');
      expect(tools.map((t) => t.tool_call_id)).toEqual(['a', 'b', 'c']);
      expect(tools[0]?.content).toBe('A2');
    });
  });
});
