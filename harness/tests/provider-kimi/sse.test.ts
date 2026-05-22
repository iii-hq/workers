import { describe, expect, it } from 'vitest';
import {
  buildPartial,
  classifyKimiError,
  emptyPartial,
  handleChunk,
  mapFinishReason,
  mergeUsage,
} from '../../src/provider-kimi/sse.js';

describe('mergeUsage (kimi)', () => {
  it('extracts chat-completions cached_tokens', () => {
    const u = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
    mergeUsage(
      {
        prompt_tokens: 1500,
        completion_tokens: 200,
        prompt_tokens_details: { cached_tokens: 1200 },
      },
      u,
    );
    expect(u.input).toBe(1500);
    expect(u.output).toBe(200);
    expect(u.cache_read).toBe(1200);
  });
});

describe('mapFinishReason (kimi)', () => {
  it('maps known finish reasons', () => {
    expect(mapFinishReason('stop')).toBe('end');
    expect(mapFinishReason('length')).toBe('length');
    expect(mapFinishReason('tool_calls')).toBe('function_call');
    expect(mapFinishReason('function_call')).toBe('function_call');
  });
});

describe('handleChunk (kimi)', () => {
  it('emits text_start on first content delta then text_delta', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { content: 'hi' } }] },
      state,
      'kimi-k2-0905-preview',
      'kimi',
    );
    expect(events.map((e) => e.type)).toEqual(['text_start', 'text_delta']);
    expect(state.text).toBe('hi');
  });

  it('accumulates tool_call arguments across chunks', () => {
    const state = emptyPartial();
    handleChunk(
      {
        choices: [
          {
            delta: {
              tool_calls: [
                { index: 0, id: 'tc1', function: { name: 'shell::exec', arguments: '{"x' } },
              ],
            },
          },
        ],
      },
      state,
      'kimi-k2-0905-preview',
      'kimi',
    );
    handleChunk(
      {
        choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: '":1}' } }] } }],
      },
      state,
      'kimi-k2-0905-preview',
      'kimi',
    );
    expect(state.tool_calls[0]?.id).toBe('tc1');
    expect(state.tool_calls[0]?.function_id).toBe('shell::exec');
    expect(state.tool_calls[0]?.args_json).toBe('{"x":1}');
  });

  it('records finish_reason as stop_reason on the partial state', () => {
    const state = emptyPartial();
    handleChunk(
      { choices: [{ finish_reason: 'tool_calls' }] },
      state,
      'kimi-k2-0905-preview',
      'kimi',
    );
    const partial = buildPartial(state, 'kimi-k2-0905-preview', 'kimi');
    expect(partial.stop_reason).toBe('function_call');
  });
});

describe('classifyKimiError', () => {
  it('maps 401/403 to auth_expired', () => {
    expect(classifyKimiError('unauthorized', 401)).toBe('auth_expired');
    expect(classifyKimiError('forbidden', 403)).toBe('auth_expired');
  });

  it('maps 429 to rate_limited', () => {
    expect(classifyKimiError('too many requests', 429)).toBe('rate_limited');
  });

  it('maps 5xx to transient', () => {
    expect(classifyKimiError('bad gateway', 502)).toBe('transient');
  });

  it('maps context length messages to context_overflow', () => {
    expect(classifyKimiError('context length exceeded', 400)).toBe('context_overflow');
  });

  it('defaults to permanent', () => {
    expect(classifyKimiError('something else', 400)).toBe('permanent');
  });
});

describe('handleChunk — Kimi K2 thinking mode reasoning_content', () => {
  // Kimi K2.6 streams reasoning tokens on `delta.reasoning_content` BEFORE
  // any content/tool_calls. If we don't capture them and don't echo back
  // on the next request, Kimi rejects: "thinking is enabled but
  // reasoning_content is missing in assistant tool call message".

  it('accumulates delta.reasoning_content into state.reasoning_text and emits thinking_start + thinking_delta', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { reasoning_content: 'Let me think about this' } }] },
      state,
      'kimi-k2.6',
      'kimi',
    );
    expect(state.reasoning_text).toBe('Let me think about this');
    expect(events.map((e) => e.type)).toEqual(['thinking_start', 'thinking_delta']);
    const delta = events[1] as Extract<(typeof events)[number], { type: 'thinking_delta' }>;
    expect(delta.delta).toBe('Let me think about this');
  });

  it('emits thinking_delta only (no second thinking_start) on subsequent reasoning chunks', () => {
    const state = emptyPartial();
    handleChunk(
      { choices: [{ delta: { reasoning_content: 'first ' } }] },
      state,
      'm',
      'kimi',
    );
    const events = handleChunk(
      { choices: [{ delta: { reasoning_content: 'second' } }] },
      state,
      'm',
      'kimi',
    );
    expect(state.reasoning_text).toBe('first second');
    expect(events.map((e) => e.type)).toEqual(['thinking_delta']);
  });

  it('persists reasoning_text as a leading thinking ContentBlock on the AssistantMessage', () => {
    const state = emptyPartial();
    handleChunk(
      { choices: [{ delta: { reasoning_content: 'plan: call shell::ls' } }] },
      state,
      'kimi-k2.6',
      'kimi',
    );
    handleChunk(
      { choices: [{ delta: { content: 'ok' } }] },
      state,
      'kimi-k2.6',
      'kimi',
    );
    const partial = buildPartial(state, 'kimi-k2.6', 'kimi');
    // Thinking must come FIRST so wire-messages can project it back as
    // reasoning_content on the assistant entry before content/tool_calls.
    expect(partial.content[0]).toEqual({ type: 'thinking', text: 'plan: call shell::ls' });
    expect(partial.content[1]).toEqual({ type: 'text', text: 'ok' });
  });

  it('handles reasoning_content + tool_calls in the same turn (the failing-on-prod shape)', () => {
    const state = emptyPartial();
    handleChunk(
      {
        choices: [
          {
            delta: {
              reasoning_content: 'I need to check the engine functions',
              tool_calls: [
                {
                  index: 0,
                  id: 'tc-1',
                  function: { name: 'directory::engine::functions::list', arguments: '{}' },
                },
              ],
            },
          },
        ],
      },
      state,
      'kimi-k2.6',
      'kimi',
    );
    const partial = buildPartial(state, 'kimi-k2.6', 'kimi');
    // Critical assertion for the bug: tool-call turn must carry the
    // thinking block, otherwise wire-messages can't echo it back as
    // reasoning_content and Kimi rejects the next request.
    const thinking = partial.content.find((c) => c.type === 'thinking') as
      | { type: 'thinking'; text: string }
      | undefined;
    expect(thinking?.text).toBe('I need to check the engine functions');
    const fcall = partial.content.find((c) => c.type === 'function_call') as
      | { type: 'function_call'; function_id: string }
      | undefined;
    expect(fcall?.function_id).toBe('directory::engine::functions::list');
  });

  it('emits nothing for thinking when reasoning_content is empty string', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { reasoning_content: '', content: 'real text' } }] },
      state,
      'm',
      'kimi',
    );
    expect(state.reasoning_text).toBe('');
    // text_start + text_delta from the real content; no thinking_* events.
    expect(events.some((e) => e.type === 'thinking_start')).toBe(false);
    expect(events.some((e) => e.type === 'thinking_delta')).toBe(false);
  });
});
