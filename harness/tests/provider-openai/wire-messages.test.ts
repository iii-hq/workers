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
});
