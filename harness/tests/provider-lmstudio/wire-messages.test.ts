import { describe, expect, it } from 'vitest';
import {
  PLACEHOLDER_USER_MESSAGE,
  toOpenaiMessages,
} from '../../src/provider-lmstudio/wire-messages.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

describe('toOpenaiMessages (lmstudio)', () => {
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
      model: 'qwen/qwen3-4b-2507',
      provider: 'lmstudio',
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

describe('toOpenaiMessages — placeholder injection for strict templates', () => {
  // qwen3's jinja template aborts with "No user query found in messages."
  // when zero `role: 'user'` messages are present. This happens in normal
  // agentic loops after a tool cycle whose original user message got
  // summarised away by async compaction. We inject a minimal "(continue)"
  // user message so the request still renders.

  it('appends a placeholder user message when no user message is present (tool-call only history)', () => {
    const assistantWithToolCall: AgentMessage = {
      role: 'assistant',
      content: [
        {
          type: 'function_call',
          id: 'tc1',
          function_id: 'shell::ls',
          arguments: { path: '/' },
        },
      ],
      stop_reason: 'function_call',
      model: 'qwen/qwen3.6-35b-a3b',
      provider: 'lmstudio',
      timestamp: 0,
    };
    const toolResult: AgentMessage = {
      role: 'function_result',
      function_call_id: 'tc1',
      function_id: 'shell::ls',
      content: [{ type: 'text', text: 'home\nroot\n' }],
      details: {},
      is_error: false,
      timestamp: 0,
    };
    const out = toOpenaiMessages(
      [assistantWithToolCall, toolResult],
      'be helpful',
    ) as Array<Record<string, unknown>>;
    // system + assistant(tool_call) + tool + placeholder user
    expect(out).toHaveLength(4);
    expect(out[3]).toEqual({ role: 'user', content: PLACEHOLDER_USER_MESSAGE });
  });

  it('appends a placeholder when messages is entirely empty (post-summary, pre-replay race)', () => {
    const out = toOpenaiMessages([], '') as Array<Record<string, unknown>>;
    expect(out).toEqual([{ role: 'user', content: PLACEHOLDER_USER_MESSAGE }]);
  });

  it('does NOT append a placeholder when a user message is already present', () => {
    const userMsg: AgentMessage = {
      role: 'user',
      content: [{ type: 'text', text: 'hi' }],
      timestamp: 0,
    };
    const out = toOpenaiMessages([userMsg], '') as Array<Record<string, unknown>>;
    expect(out).toHaveLength(1);
    expect(out[0]).toEqual({ role: 'user', content: 'hi' });
  });

  it('does NOT count system_prompt as a user message — placeholder still appends if no real user msg follows', () => {
    const assistantOnly: AgentMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: 'reply' }],
      stop_reason: 'end',
      model: 'm',
      provider: 'lmstudio',
      timestamp: 0,
    };
    const out = toOpenaiMessages(
      [assistantOnly],
      'be terse',
    ) as Array<Record<string, unknown>>;
    expect(out).toHaveLength(3);
    expect(out[0]?.role).toBe('system');
    expect(out[1]?.role).toBe('assistant');
    expect(out[2]).toEqual({ role: 'user', content: PLACEHOLDER_USER_MESSAGE });
  });
});

describe('toOpenaiMessages (lmstudio) — round-tripping reasoning_content', () => {
  // Mirror of the Kimi fix: thinking-mode GGUFs served by LM Studio
  // (qwen3, glm, deepseek-r1) need `reasoning_content` echoed back on
  // assistant tool-call messages or they fail with the same template
  // error Kimi K2.6 produces.

  it('emits reasoning_content when the assistant has a thinking block + tool_calls', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        { type: 'thinking', text: 'I will list /' },
        {
          type: 'function_call',
          id: 'tc-1',
          function_id: 'shell::ls',
          arguments: { path: '/' },
        },
      ],
      stop_reason: 'function_call',
      model: 'qwen/qwen3-thinking',
      provider: 'lmstudio',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    expect(out[0]?.reasoning_content).toBe('I will list /');
    expect(out[0]?.tool_calls).toBeDefined();
  });

  it('omits reasoning_content for non-thinking models (no thinking block)', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: 'hi' }],
      stop_reason: 'end',
      model: 'qwen/qwen3-4b-2507',
      provider: 'lmstudio',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    expect(out[0]?.reasoning_content).toBeUndefined();
  });
});

describe('toOpenaiMessages (lmstudio) — boundary dedup of duplicate tool messages', () => {
  // Same defense as the other providers — LM Studio's jinja templates
  // vary by GGUF; some accept duplicate tool messages, some don't.
  // Latest-wins replace keeps every template happy.

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
    expect(tools[0]?.content).toBe('second');
  });
});
