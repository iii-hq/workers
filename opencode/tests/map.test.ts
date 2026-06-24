import { describe, expect, it } from 'vitest';
import {
  addUsage,
  makeAssistantMessage,
  makeFunctionResult,
  mapToolOutput,
  toolFunctionId,
} from '../src/map.js';

describe('toolFunctionId', () => {
  it('namespaces built-in OpenCode tools', () => {
    expect(toolFunctionId('bash')).toBe('opencode::bash');
    expect(toolFunctionId('edit')).toBe('opencode::edit');
  });
  it('maps MCP tool names to bus-style ids', () => {
    expect(toolFunctionId('mcp__github__create_issue')).toBe('github::create_issue');
  });
});

describe('mapToolOutput', () => {
  it('wraps a string as one text block', () => {
    expect(mapToolOutput('hi\n')).toEqual([{ type: 'text', text: 'hi\n' }]);
  });
  it('stringifies objects and empties null', () => {
    expect(mapToolOutput({ ok: true })).toEqual([{ type: 'text', text: '{"ok":true}' }]);
    expect(mapToolOutput(null)).toEqual([]);
  });
});

describe('addUsage', () => {
  it('maps OpenCode step tokens onto the wire shape', () => {
    const u = addUsage(null, { input: 3, output: 5, reasoning: 1, cache: { read: 10, write: 20 } });
    expect(u).toEqual({
      input_tokens: 3,
      output_tokens: 5,
      reasoning_tokens: 1,
      cache_read_tokens: 10,
      cache_write_tokens: 20,
    });
  });
  it('accumulates across steps', () => {
    let u = addUsage(null, { input: 1, output: 2 });
    u = addUsage(u, { input: 4, output: 8, cache: { read: 5 } });
    expect(u.input_tokens).toBe(5);
    expect(u.output_tokens).toBe(10);
    expect(u.cache_read_tokens).toBe(5);
  });
  it('tolerates missing tokens', () => {
    expect(addUsage(null, undefined).input_tokens).toBe(0);
  });
});

describe('message constructors', () => {
  it('builds an assistant message with provider opencode', () => {
    const m = makeAssistantMessage([{ type: 'text', text: 'hi' }], 'anthropic/claude', null);
    expect(m.role).toBe('assistant');
    expect(m.provider).toBe('opencode');
    expect(m.stop_reason).toBe('end');
  });
  it('builds a function_result message', () => {
    const fr = makeFunctionResult(
      'toolu_1',
      'opencode::bash',
      [{ type: 'text', text: 'ok' }],
      false,
    );
    expect(fr.role).toBe('function_result');
    expect(fr.function_call_id).toBe('toolu_1');
    expect(fr.is_error).toBe(false);
  });
});
