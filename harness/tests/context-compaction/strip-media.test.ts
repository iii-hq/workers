import { describe, expect, it } from 'vitest';
import { stripMedia } from '../../src/context-compaction/strip-media.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

function user(): AgentMessage {
  return {
    role: 'user',
    content: [
      { type: 'text', text: 'hi' },
      // ContentBlock allows image blocks; use a structural shape.
      { type: 'image' } as unknown as { type: 'text'; text: string },
    ],
    timestamp: 0,
  };
}

function bigToolResult(): AgentMessage {
  return {
    role: 'function_result',
    function_call_id: 'fc1',
    function_id: 'shell::fs::cat',
    content: [{ type: 'text', text: 'x'.repeat(10_000) }],
    details: {},
    is_error: false,
    timestamp: 0,
  };
}

describe('stripMedia', () => {
  it('replaces image content blocks with text placeholders', () => {
    const out = stripMedia([user()], { toolOutputMaxChars: 2000 });
    expect(out[0]?.content).toEqual([
      { type: 'text', text: 'hi' },
      { type: 'text', text: '[image stripped]' },
    ]);
  });
  it('truncates oversized tool outputs', () => {
    const out = stripMedia([bigToolResult()], { toolOutputMaxChars: 100 });
    const r = out[0];
    if (r?.role !== 'function_result') throw new Error('role drift');
    const block = r.content[0] as { type: 'text'; text: string };
    expect(block.text.length).toBeLessThanOrEqual(100 + 32);
    expect(block.text.endsWith('[truncated]')).toBe(true);
  });
  it('leaves short tool outputs untouched', () => {
    const m: AgentMessage = {
      role: 'function_result',
      function_call_id: 'fc',
      function_id: 'x',
      content: [{ type: 'text', text: 'short' }],
      details: {},
      is_error: false,
      timestamp: 0,
    };
    const out = stripMedia([m], { toolOutputMaxChars: 2000 });
    expect(out[0]?.content[0]).toEqual({ type: 'text', text: 'short' });
  });
  it('strips images inside function_result content', () => {
    const m: AgentMessage = {
      role: 'function_result',
      function_call_id: 'fc',
      function_id: 'screenshot',
      content: [
        { type: 'text', text: 'capture' },
        { type: 'image' } as unknown as { type: 'text'; text: string },
      ],
      details: {},
      is_error: false,
      timestamp: 0,
    };
    const out = stripMedia([m], { toolOutputMaxChars: 2000 });
    expect(out[0]?.content).toEqual([
      { type: 'text', text: 'capture' },
      { type: 'text', text: '[image stripped]' },
    ]);
  });
});
