import { describe, expect, it } from 'vitest';
import {
  contentBlockToWire,
  decodeToolName,
  encodeToolName,
  toWireMessages,
} from '../../src/provider-anthropic/wire-messages.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

describe('encode/decodeToolName', () => {
  it('replaces :: with __', () => {
    expect(encodeToolName('shell::fs::ls')).toBe('shell__fs__ls');
    expect(decodeToolName('shell__fs__ls')).toBe('shell::fs::ls');
  });
});

describe('contentBlockToWire', () => {
  it('encodes function_call as tool_use with encoded name', () => {
    const out = contentBlockToWire({
      type: 'function_call',
      id: 'tc1',
      function_id: 'shell::exec',
      arguments: { x: 1 },
    });
    expect(out).toEqual({
      type: 'tool_use',
      id: 'tc1',
      name: 'shell__exec',
      input: { x: 1 },
    });
  });

  it('skips other block kinds', () => {
    expect(
      contentBlockToWire({
        type: 'image',
        mime: 'image/png',
        data: 'aGk=',
      }),
    ).toBeNull();
  });
});

describe('toWireMessages', () => {
  it('converts user message to wire user', () => {
    const msgs: AgentMessage[] = [
      { role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: 0 },
    ];
    const wire = toWireMessages(msgs) as Array<Record<string, unknown>>;
    expect(wire[0]?.role).toBe('user');
    expect((wire[0] as { content: Array<{ type: string }> }).content[0]?.type).toBe('text');
  });

  it('collapses parallel function_results into one user message with tool_result blocks', () => {
    const mk = (id: string): AgentMessage => ({
      role: 'function_result',
      function_call_id: id,
      function_id: 'read',
      content: [{ type: 'text', text: id }],
      details: {},
      is_error: false,
      timestamp: 0,
    });
    const wire = toWireMessages([mk('a'), mk('b'), mk('c')]) as Array<Record<string, unknown>>;
    expect(wire.length).toBe(1);
    const content = (wire[0] as { content: Array<{ tool_use_id: string }> }).content;
    expect(content).toHaveLength(3);
    expect(content[0]?.tool_use_id).toBe('a');
    expect(content[2]?.tool_use_id).toBe('c');
  });

  it('skips custom messages', () => {
    const msgs: AgentMessage[] = [
      {
        role: 'custom',
        custom_type: 'note',
        content: [{ type: 'text', text: 'x' }],
        timestamp: 0,
      },
    ];
    expect(toWireMessages(msgs)).toEqual([]);
  });
});
