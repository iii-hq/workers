import { describe, expect, it } from 'vitest';
import {
  lastAssistant,
  makeAssistantMessage,
  makeFunctionResult,
  mapMessageContent,
  mapToolResultContent,
  mapUsage,
  toolFunctionId,
} from '../src/map.js';
import type { AgentMessage } from '../src/types.js';

describe('toolFunctionId', () => {
  it('namespaces built-in Pi tools', () => {
    expect(toolFunctionId('bash')).toBe('pi::bash');
    expect(toolFunctionId('edit')).toBe('pi::edit');
  });

  it('maps MCP tool names to bus-style ids', () => {
    expect(toolFunctionId('mcp__github__create_issue')).toBe('github::create_issue');
    expect(toolFunctionId('mcp__filesystem__read_file')).toBe('filesystem::read_file');
  });
});

describe('mapMessageContent', () => {
  it('extracts text and thinking blocks', () => {
    const out = mapMessageContent({
      role: 'assistant',
      content: [
        { type: 'text', text: 'hello' },
        { type: 'thinking', thinking: 'hmm' },
      ],
    });
    expect(out).toEqual([
      { type: 'text', text: 'hello' },
      { type: 'thinking', text: 'hmm' },
    ]);
  });

  it('reads reasoning as thinking', () => {
    const out = mapMessageContent({ content: [{ type: 'reasoning', reasoning: 'because' }] });
    expect(out).toEqual([{ type: 'thinking', text: 'because' }]);
  });

  it('wraps string content as one text block', () => {
    expect(mapMessageContent({ content: 'plain' })).toEqual([{ type: 'text', text: 'plain' }]);
  });

  it('drops tool-call blocks (surfaced as tool_execution events) and empties', () => {
    const out = mapMessageContent({
      content: [{ type: 'toolCall', id: 'x' }, { type: 'text' }],
    });
    expect(out).toEqual([]);
  });

  it('returns nothing for non-message input', () => {
    expect(mapMessageContent(null)).toEqual([]);
    expect(mapMessageContent({ content: 42 })).toEqual([]);
  });
});

describe('mapToolResultContent', () => {
  it('wraps strings as one text block', () => {
    expect(mapToolResultContent('ok')).toEqual([{ type: 'text', text: 'ok' }]);
  });

  it('flattens block arrays to text', () => {
    expect(mapToolResultContent([{ type: 'text', text: 'a' }, { type: 'image' }])).toEqual([
      { type: 'text', text: 'a' },
      { type: 'text', text: '{"type":"image"}' },
    ]);
  });

  it('preserves scalar array entries instead of dropping them', () => {
    expect(mapToolResultContent(['plain', 42, null])).toEqual([
      { type: 'text', text: '"plain"' },
      { type: 'text', text: '42' },
      { type: 'text', text: 'null' },
    ]);
  });

  it('stringifies anything else', () => {
    expect(mapToolResultContent({ ok: true })).toEqual([{ type: 'text', text: '{"ok":true}' }]);
    expect(mapToolResultContent(undefined)).toEqual([{ type: 'text', text: 'null' }]);
  });
});

describe('mapUsage', () => {
  it('maps Pi token fields onto the wire shape', () => {
    expect(mapUsage({ input: 10, output: 5, cacheRead: 100, cacheWrite: 7, total: 122 })).toEqual({
      input_tokens: 10,
      output_tokens: 5,
      cache_read_tokens: 100,
      cache_write_tokens: 7,
    });
  });

  it('returns null for non-objects', () => {
    expect(mapUsage(null)).toBeNull();
    expect(mapUsage('x')).toBeNull();
  });

  it('defaults absent cache token fields to 0 instead of undefined', () => {
    expect(mapUsage({ input: 3, output: 1 })).toEqual({
      input_tokens: 3,
      output_tokens: 1,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
    });
  });
});

describe('message constructors', () => {
  it('builds an assistant message with provider pi', () => {
    const msg = makeAssistantMessage([{ type: 'text', text: 'hi' }], 'anthropic/claude', null);
    expect(msg.role).toBe('assistant');
    expect(msg.provider).toBe('pi');
    expect(msg.stop_reason).toBe('end');
  });

  it('builds a function_result message', () => {
    const fr = makeFunctionResult('call_1', 'pi::bash', [{ type: 'text', text: 'ok' }], false);
    expect(fr.role).toBe('function_result');
    expect(fr.function_call_id).toBe('call_1');
    expect(fr.is_error).toBe(false);
  });
});

describe('lastAssistant', () => {
  it('returns the last assistant message', () => {
    const a1 = makeAssistantMessage([{ type: 'text', text: 'one' }], 'm', null);
    const a2 = makeAssistantMessage([{ type: 'text', text: 'two' }], 'm', null);
    const fr = makeFunctionResult('id', 'fn', [], false);
    const messages: AgentMessage[] = [a1, a2, fr];
    expect(lastAssistant(messages)).toBe(a2);
  });

  it('falls back to the final message when no assistant exists', () => {
    const fr = makeFunctionResult('id', 'fn', [], false);
    expect(lastAssistant([fr])).toBe(fr);
  });

  it('returns a synthetic assistant message for an empty transcript', () => {
    const msg = lastAssistant([]);
    expect(msg).toMatchObject({ role: 'assistant', content: [] });
  });
});
