import { describe, expect, it } from 'vitest';
import {
  classifyLlamacppError,
  emptyPartial,
  extractErrorMessage,
  handleChunk,
  mapFinishReason,
  syntheticErrorEvent,
} from '../../src/provider-llamacpp/sse.js';

describe('mapFinishReason', () => {
  it.each([
    ['stop', 'end'],
    ['length', 'length'],
    ['tool_calls', 'function_call'],
    ['function_call', 'function_call'],
    ['somethingelse', 'end'],
  ])('%s → %s', (input, expected) => {
    expect(mapFinishReason(input)).toBe(expected);
  });
});

describe('extractErrorMessage', () => {
  it('returns null when no error field', () => {
    expect(extractErrorMessage({ choices: [] })).toBeNull();
  });

  it('handles `error` as a string', () => {
    expect(extractErrorMessage({ error: 'bad' })).toBe('bad');
  });

  it('handles nested `error.message`', () => {
    expect(extractErrorMessage({ error: { message: 'rate-limited' } })).toBe('rate-limited');
  });

  it('handles nested `error.detail`', () => {
    expect(extractErrorMessage({ error: { detail: 'bad request' } })).toBe('bad request');
  });
});

describe('classifyLlamacppError', () => {
  it('maps 401/403 to auth_expired', () => {
    expect(classifyLlamacppError('forbidden', 401)).toBe('auth_expired');
    expect(classifyLlamacppError('forbidden', 403)).toBe('auth_expired');
  });

  it('maps 429 to rate_limited', () => {
    expect(classifyLlamacppError('slow down', 429)).toBe('rate_limited');
  });

  it('maps 5xx to transient', () => {
    expect(classifyLlamacppError('boom', 502)).toBe('transient');
  });

  it('detects context_overflow via message', () => {
    expect(classifyLlamacppError('context length exceeded')).toBe('context_overflow');
    expect(classifyLlamacppError('n_ctx exceeded')).toBe('context_overflow');
  });

  it('falls through to permanent', () => {
    expect(classifyLlamacppError('something else')).toBe('permanent');
  });
});

describe('syntheticErrorEvent', () => {
  it('puts the message in error_message ONLY (content stays empty)', () => {
    const ev = syntheticErrorEvent('boom', 'm', 'llamacpp', 'transient');
    expect(ev.type).toBe('error');
    if (ev.type !== 'error') throw new Error('unreachable');
    expect(ev.error.error_message).toBe('boom');
    expect(ev.error.content).toEqual([]);
    expect(ev.error.error_kind).toBe('transient');
  });
});

describe('handleChunk', () => {
  it('emits text_start + text_delta on first content delta', () => {
    const state = emptyPartial();
    const events = handleChunk({ choices: [{ delta: { content: 'Hi' } }] }, state, 'm', 'llamacpp');
    expect(events.map((e) => e.type)).toEqual(['text_start', 'text_delta']);
  });

  it('only emits text_start once across deltas', () => {
    const state = emptyPartial();
    handleChunk({ choices: [{ delta: { content: 'A' } }] }, state, 'm', 'llamacpp');
    const events = handleChunk({ choices: [{ delta: { content: 'B' } }] }, state, 'm', 'llamacpp');
    expect(events.map((e) => e.type)).toEqual(['text_delta']);
  });

  it('emits thinking events on reasoning_content', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { reasoning_content: 'planning' } }] },
      state,
      'm',
      'llamacpp',
    );
    expect(events.map((e) => e.type)).toEqual(['thinking_start', 'thinking_delta']);
  });

  it('records finish_reason and stop_reason mapping', () => {
    const state = emptyPartial();
    handleChunk({ choices: [{ finish_reason: 'length', delta: {} }] }, state, 'm', 'llamacpp');
    expect(state.saw_finish_reason).toBe(true);
    expect(state.stop_reason).toBe('length');
  });

  it('rejects attacker-controlled tool_calls indices (DoS guard)', () => {
    const state = emptyPartial();
    handleChunk(
      {
        choices: [
          {
            delta: {
              tool_calls: [{ index: 1e9, id: 't1', function: { name: 'x' } }],
            },
          },
        ],
      },
      state,
      'm',
      'llamacpp',
    );
    // The unbounded index is dropped — tool_calls length stays 0.
    expect(state.tool_calls.length).toBe(0);
  });

  it('returns an error event when the chunk carries an `error` field', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { error: { message: 'No user query found' } },
      state,
      'm',
      'llamacpp',
    );
    expect(events).toHaveLength(1);
    expect(events[0]?.type).toBe('error');
    expect(state.saw_finish_reason).toBe(true);
    expect(state.stop_reason).toBe('error');
  });
});
