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
