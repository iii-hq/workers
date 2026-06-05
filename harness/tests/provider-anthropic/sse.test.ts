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

describe('handleSseEvent thinking blocks', () => {
  it('emits thinking_start + thinking_delta + thinking_end in order', () => {
    const state = emptyPartial();
    const out: string[] = [];
    out.push(
      ...handleSseEvent(
        'data: {"type":"content_block_start","content_block":{"type":"thinking"}}',
        state,
        'm',
      ).map((e) => e.type),
    );
    out.push(
      ...handleSseEvent(
        'data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"let me think"}}',
        state,
        'm',
      ).map((e) => e.type),
    );
    out.push(
      ...handleSseEvent('data: {"type":"content_block_stop"}', state, 'm').map((e) => e.type),
    );
    expect(out).toEqual(['thinking_start', 'thinking_delta', 'thinking_end']);
    expect(state.thinking_blocks[0]?.text).toBe('let me think');
  });

  it('accumulates signature_delta without emitting events', () => {
    const state = emptyPartial();
    handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"thinking"}}',
      state,
      'm',
    );
    const events = handleSseEvent(
      'data: {"type":"content_block_delta","delta":{"type":"signature_delta","signature":"sig123"}}',
      state,
      'm',
    );
    expect(events).toEqual([]);
    expect(state.thinking_blocks[0]?.signature).toBe('sig123');
  });

  it('persists a leading ThinkingContent block in the assembled message', () => {
    const state = emptyPartial();
    handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"thinking"}}',
      state,
      'm',
    );
    handleSseEvent(
      'data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"why"}}',
      state,
      'm',
    );
    handleSseEvent('data: {"type":"content_block_stop"}', state, 'm');
    state.text_blocks.push('answer');
    const partial = buildPartial(state, 'm');
    expect(partial.content).toEqual([
      { type: 'thinking', text: 'why' },
      { type: 'text', text: 'answer' },
    ]);
  });

  it('emits the end event matching the open block kind (regression: tool_use end)', () => {
    const state = emptyPartial();
    handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":"x"}}',
      state,
      'm',
    );
    const stop = handleSseEvent('data: {"type":"content_block_stop"}', state, 'm');
    expect(stop[0]?.type).toBe('functioncall_end');
  });

  it('text block after a thinking block still emits text_end', () => {
    const state = emptyPartial();
    handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"thinking"}}',
      state,
      'm',
    );
    handleSseEvent('data: {"type":"content_block_stop"}', state, 'm');
    handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"text"}}',
      state,
      'm',
    );
    const stop = handleSseEvent('data: {"type":"content_block_stop"}', state, 'm');
    expect(stop[0]?.type).toBe('text_end');
  });

  it('preserves interleaved thinking/tool_use arrival order in the assembled message', () => {
    const state = emptyPartial();
    const feed = (line: string) => handleSseEvent(`data: ${line}`, state, 'm');
    // thinking₁ → tool_use₁ → thinking₂ → tool_use₂ (interleaved-thinking beta shape)
    feed('{"type":"content_block_start","content_block":{"type":"thinking"}}');
    feed('{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"plan A"}}');
    feed('{"type":"content_block_stop"}');
    feed('{"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":"x"}}');
    feed('{"type":"content_block_stop"}');
    feed('{"type":"content_block_start","content_block":{"type":"thinking"}}');
    feed('{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"plan B"}}');
    feed('{"type":"content_block_stop"}');
    feed('{"type":"content_block_start","content_block":{"type":"tool_use","id":"t2","name":"y"}}');
    feed('{"type":"content_block_stop"}');
    const partial = buildPartial(state, 'm');
    expect(partial.content.map((b) => b.type)).toEqual([
      'thinking',
      'function_call',
      'thinking',
      'function_call',
    ]);
    expect(partial.content[0]).toMatchObject({ text: 'plan A' });
    expect(partial.content[2]).toMatchObject({ text: 'plan B' });
  });

  it('redacted_thinking does not crash and emits thinking events', () => {
    const state = emptyPartial();
    const start = handleSseEvent(
      'data: {"type":"content_block_start","content_block":{"type":"redacted_thinking","data":"opaque"}}',
      state,
      'm',
    );
    expect(start[0]?.type).toBe('thinking_start');
    const stop = handleSseEvent('data: {"type":"content_block_stop"}', state, 'm');
    expect(stop[0]?.type).toBe('thinking_end');
    // Opaque content is not persisted.
    expect(buildPartial(state, 'm').content).toEqual([]);
  });
});
