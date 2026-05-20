import { describe, expect, it } from 'vitest';
import {
  buildPartial,
  emptyPartial,
  handleChunk,
  mapFinishReason,
  mergeUsage,
} from '../../src/provider-openai/sse.js';

describe('mergeUsage', () => {
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

  it('extracts responses-API cached_tokens', () => {
    const u = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
    mergeUsage(
      {
        input_tokens: 2000,
        output_tokens: 100,
        input_tokens_details: { cached_tokens: 1700 },
      },
      u,
    );
    expect(u.cache_read).toBe(1700);
  });
});

describe('mapFinishReason', () => {
  it('maps known finish reasons', () => {
    expect(mapFinishReason('stop')).toBe('end');
    expect(mapFinishReason('length')).toBe('length');
    expect(mapFinishReason('tool_calls')).toBe('function_call');
    expect(mapFinishReason('function_call')).toBe('function_call');
  });
});

describe('handleChunk', () => {
  it('emits text_start on first content delta then text_delta', () => {
    const state = emptyPartial();
    const events = handleChunk(
      {
        choices: [{ delta: { content: 'hello' } }],
      },
      state,
      'gpt-5',
      'openai',
    );
    expect(events.map((e) => e.type)).toEqual(['text_start', 'text_delta']);
    expect(state.text).toBe('hello');
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
      'gpt-5',
      'openai',
    );
    handleChunk(
      {
        choices: [
          {
            delta: {
              tool_calls: [{ index: 0, function: { arguments: '":1}' } }],
            },
          },
        ],
      },
      state,
      'gpt-5',
      'openai',
    );
    expect(state.tool_calls[0]?.id).toBe('tc1');
    expect(state.tool_calls[0]?.function_id).toBe('shell::exec');
    expect(state.tool_calls[0]?.args_json).toBe('{"x":1}');
  });

  it('records finish_reason as stop_reason on the partial state', () => {
    const state = emptyPartial();
    handleChunk({ choices: [{ finish_reason: 'tool_calls' }] }, state, 'gpt-5', 'openai');
    expect(state.stop_reason).toBe('function_call');
    const partial = buildPartial(state, 'gpt-5', 'openai');
    expect(partial.stop_reason).toBe('function_call');
  });
});
