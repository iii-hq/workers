import { describe, expect, it } from 'vitest';
import { toOpenaiMessages } from '../../src/provider-kimi/wire-messages.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

describe('toOpenaiMessages (kimi)', () => {
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
      model: 'kimi-k2-0905-preview',
      provider: 'kimi',
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

describe('toOpenaiMessages (kimi) — round-tripping reasoning_content', () => {
  // The bug this fixes: Kimi K2.6 thinking mode requires `reasoning_content`
  // on every assistant message that has tool_calls. Without it Kimi rejects:
  //   "thinking is enabled but reasoning_content is missing in assistant
  //    tool call message at index N"
  // sse.ts captures `delta.reasoning_content` into a ThinkingContent block;
  // wire-messages.ts (this code) must project it back as `reasoning_content`.

  it('emits reasoning_content on assistant messages whose content has a thinking block + tool_calls', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        { type: 'thinking', text: 'I need to list the engine functions first.' },
        {
          type: 'function_call',
          id: 'tc-1',
          function_id: 'directory::engine::functions::list',
          arguments: {},
        },
      ],
      stop_reason: 'function_call',
      model: 'kimi-k2.6',
      provider: 'kimi',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    const entry = out[0] as Record<string, unknown>;
    expect(entry.role).toBe('assistant');
    expect(entry.reasoning_content).toBe('I need to list the engine functions first.');
    expect(entry.tool_calls).toBeDefined();
  });

  it('preserves reasoning_content + text + tool_calls together (the full assistant turn shape)', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        { type: 'thinking', text: 'plan: A then B' },
        { type: 'text', text: 'On it.' },
        {
          type: 'function_call',
          id: 'tc-1',
          function_id: 'shell::ls',
          arguments: { path: '/' },
        },
      ],
      stop_reason: 'function_call',
      model: 'kimi-k2.6',
      provider: 'kimi',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    const entry = out[0] as Record<string, unknown>;
    expect(entry.reasoning_content).toBe('plan: A then B');
    expect(entry.content).toBe('On it.');
    expect(entry.tool_calls).toBeDefined();
  });

  it('omits reasoning_content when no thinking block is present (non-thinking models)', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [{ type: 'text', text: 'plain reply' }],
      stop_reason: 'end',
      model: 'kimi-k2-0905-preview',
      provider: 'kimi',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    const entry = out[0] as Record<string, unknown>;
    expect(entry.reasoning_content).toBeUndefined();
  });

  it('concatenates multiple thinking blocks (rare, but defensive)', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        { type: 'thinking', text: 'first thought. ' },
        { type: 'thinking', text: 'second thought.' },
        { type: 'text', text: 'reply' },
      ],
      stop_reason: 'end',
      model: 'kimi-k2.6',
      provider: 'kimi',
      timestamp: 0,
    };
    const out = toOpenaiMessages([msg], '') as Array<Record<string, unknown>>;
    expect((out[0] as Record<string, unknown>).reasoning_content).toBe(
      'first thought. second thought.',
    );
  });

  it('end-to-end roundtrip: 3-message conversation with assistant tool_calls carries reasoning_content', () => {
    // This is the exact shape that was failing in prod:
    //   index 0: system
    //   index 1: user
    //   index 2: assistant{tool_calls} — MUST carry reasoning_content
    //   index 3: tool result
    const messages: AgentMessage[] = [
      { role: 'user', content: [{ type: 'text', text: 'create a sandbox' }], timestamp: 0 },
      {
        role: 'assistant',
        content: [
          { type: 'thinking', text: 'first I check what exists' },
          {
            type: 'function_call',
            id: 'tc-a',
            function_id: 'directory::skills::get',
            arguments: {},
          },
        ],
        stop_reason: 'function_call',
        model: 'kimi-k2.6',
        provider: 'kimi',
        timestamp: 0,
      },
      {
        role: 'function_result',
        function_call_id: 'tc-a',
        function_id: 'directory::skills::get',
        content: [{ type: 'text', text: '[]' }],
        details: {},
        is_error: false,
        timestamp: 0,
      },
    ];
    const out = toOpenaiMessages(messages, 'you are an agent') as Array<Record<string, unknown>>;
    // [system, user, assistant, tool]
    expect(out).toHaveLength(4);
    expect(out[0]?.role).toBe('system');
    expect(out[1]?.role).toBe('user');
    expect(out[2]?.role).toBe('assistant');
    expect(out[2]?.reasoning_content).toBe('first I check what exists');
    expect(out[2]?.tool_calls).toBeDefined();
    expect(out[3]?.role).toBe('tool');
  });
});

describe('toOpenaiMessages (kimi) — boundary dedup of duplicate tool messages', () => {
  // Same defense as the Anthropic and OpenAI providers — never ship two
  // tool messages with the same tool_call_id. Kimi's strict templates in
  // K2.6 thinking mode reject this kind of duplicate.

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
