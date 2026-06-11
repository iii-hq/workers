import { describe, expect, it } from 'vitest';
import {
  lastAssistant,
  makeAssistantMessage,
  makeFunctionResult,
  mapAssistantContent,
  mapToolResultContent,
  mapUsage,
  type ToolCallIndex,
  toolFunctionId,
} from '../src/map.js';
import type { AgentMessage } from '../src/types.js';

describe('toolFunctionId', () => {
  it('namespaces built-in Claude Code tools', () => {
    expect(toolFunctionId('Bash')).toBe('claude-code::Bash');
    expect(toolFunctionId('Edit')).toBe('claude-code::Edit');
  });

  it('maps MCP tool names to bus-style ids', () => {
    expect(toolFunctionId('mcp__github__create_issue')).toBe('github::create_issue');
    expect(toolFunctionId('mcp__filesystem__read_file')).toBe('filesystem::read_file');
  });
});

describe('mapAssistantContent', () => {
  it('maps text, thinking, and tool_use blocks and indexes calls', () => {
    const calls: ToolCallIndex = new Map();
    const blocks = [
      { type: 'text', text: 'hello' },
      { type: 'thinking', thinking: 'hmm' },
      { type: 'tool_use', id: 'toolu_1', name: 'Bash', input: { command: 'ls' } },
    ];
    const out = mapAssistantContent(blocks, calls);
    expect(out).toEqual([
      { type: 'text', text: 'hello' },
      { type: 'thinking', text: 'hmm' },
      {
        type: 'function_call',
        id: 'toolu_1',
        function_id: 'claude-code::Bash',
        arguments: { command: 'ls' },
      },
    ]);
    expect(calls.get('toolu_1')?.function_id).toBe('claude-code::Bash');
  });

  it('drops empty and unknown blocks', () => {
    const calls: ToolCallIndex = new Map();
    expect(mapAssistantContent([{ type: 'text' }, { type: 'mystery' }], calls)).toEqual([]);
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
  it('maps SDK usage fields onto the wire shape', () => {
    expect(
      mapUsage({
        input_tokens: 10,
        output_tokens: 5,
        cache_read_input_tokens: 100,
        cache_creation_input_tokens: 7,
      }),
    ).toEqual({
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
});

describe('message constructors', () => {
  it('builds an assistant message with provider claude-code', () => {
    const msg = makeAssistantMessage([{ type: 'text', text: 'hi' }], 'claude-opus-4-8', null);
    expect(msg.role).toBe('assistant');
    expect(msg.provider).toBe('claude-code');
    expect(msg.stop_reason).toBe('end');
  });

  it('builds a function_result message', () => {
    const fr = makeFunctionResult(
      'toolu_1',
      'claude-code::Bash',
      [{ type: 'text', text: 'ok' }],
      false,
    );
    expect(fr.role).toBe('function_result');
    expect(fr.function_call_id).toBe('toolu_1');
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
