import { describe, expect, it } from 'vitest';
import {
  buildPartial,
  emptyPartial,
  handleSseEvent,
  mapStopReason,
  mergeUsage,
} from '../../src/provider-anthropic/sse.js';

describe('mapStopReason', () => {
  it('maps known values', () => {
    expect(mapStopReason('end_turn')).toBe('end');
    expect(mapStopReason('max_tokens')).toBe('length');
    expect(mapStopReason('tool_use')).toBe('function_call');
    expect(mapStopReason('weird')).toBe('end');
  });
});

describe('mergeUsage', () => {
  it('accumulates input + output + cache fields', () => {
    const u = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
    mergeUsage({ input_tokens: 10, output_tokens: 20 }, u);
    mergeUsage(
      {
        input_tokens: 5,
        output_tokens: 6,
        cache_read_input_tokens: 80,
        cache_creation_input_tokens: 20,
      },
      u,
    );
    expect(u.input).toBe(15);
    expect(u.output).toBe(26);
    expect(u.cache_read).toBe(80);
    expect(u.cache_write).toBe(20);
  });
});

describe('handleSseEvent', () => {
  it('emits text_start + text_delta + text_end on a text content block', () => {
    const state = emptyPartial();
    const out: string[] = [];
    out.push(
      ...handleSseEvent(
        'data: {"type":"content_block_start","content_block":{"type":"text"}}',
        state,
        'm',
      ).map((e) => e.type),
    );
    out.push(
      ...handleSseEvent(
        'data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}',
        state,
        'm',
      ).map((e) => e.type),
    );
    out.push(
      ...handleSseEvent('data: {"type":"content_block_stop"}', state, 'm').map((e) => e.type),
    );
    expect(out).toEqual(['text_start', 'text_delta', 'text_end']);
    expect(state.text_blocks).toEqual(['hello']);
  });

  it('accumulates input_json_delta into the latest function call', () => {
    const state = emptyPartial();
    handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"tc1","name":"shell__fs__ls"}}',
      state,
      'm',
    );
    handleSseEvent(
      'data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\\"path"}}',
      state,
      'm',
    );
    handleSseEvent(
      'data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"\\":\\"/tmp\\"}"}}',
      state,
      'm',
    );
    expect(state.function_calls[0]?.id).toBe('tc1');
    expect(state.function_calls[0]?.function_id).toBe('shell::fs::ls');
    expect(state.function_calls[0]?.args_json).toBe('{"path":"/tmp"}');
  });

  it('records stop_reason from message_delta and emits stop event', () => {
    const state = emptyPartial();
    handleSseEvent('data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}', state, 'm');
    expect(state.stop_reason).toBe('function_call');
    const events = handleSseEvent('data: {"type":"message_stop"}', state, 'm');
    expect(events[0]?.type).toBe('stop');
  });

  it('returns no events on unparseable SSE', () => {
    const state = emptyPartial();
    expect(handleSseEvent('event: bad', state, 'm')).toEqual([]);
    expect(handleSseEvent('data: not-json', state, 'm')).toEqual([]);
  });

  it('buildPartial reflects accumulated text', () => {
    const state = emptyPartial();
    state.text_blocks.push('hello');
    const partial = buildPartial(state, 'm');
    expect(partial.content[0]).toEqual({ type: 'text', text: 'hello' });
  });
});
