import { describe, expect, it } from 'vitest';
import {
  estimateTokenCount,
  renderUserPrompt,
  splitMessages,
} from '../../src/context-compaction/summarize.js';
import type { AgentMessage } from '../../src/types/agent-message.js';

const text = (t: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text: t }],
  timestamp: 0,
});

describe('splitMessages', () => {
  it('keeps all when below threshold', () => {
    const r = splitMessages([text('a'), text('b')], 3);
    expect(r.older).toEqual([]);
    expect(r.recent).toHaveLength(2);
  });

  it('splits at the keep boundary', () => {
    const msgs = Array.from({ length: 10 }, (_, i) => text(String(i)));
    const r = splitMessages(msgs, 3);
    expect(r.older).toHaveLength(7);
    expect(r.recent).toHaveLength(3);
  });
});

describe('estimateTokenCount', () => {
  it('scales with message size', () => {
    expect(estimateTokenCount([text('x'.repeat(4000))])).toBeGreaterThan(
      estimateTokenCount([text('x')]),
    );
  });
});

describe('renderUserPrompt', () => {
  it('includes role and text', () => {
    const out = renderUserPrompt([
      text('hello'),
      {
        role: 'assistant',
        content: [{ type: 'text', text: 'hi' }],
        stop_reason: 'end',
        model: 'm',
        provider: 'p',
        timestamp: 0,
      },
    ]);
    expect(out).toContain('[user]');
    expect(out).toContain('[assistant]');
    expect(out).toContain('hello');
    expect(out).toContain('hi');
    expect(out).toContain('<conversation>');
  });

  it('renders function_call blocks', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        {
          type: 'function_call',
          id: 'tc1',
          function_id: 'shell::run',
          arguments: { command: 'ls' },
        },
      ],
      stop_reason: 'function_call',
      model: 'm',
      provider: 'p',
      timestamp: 0,
    };
    const out = renderUserPrompt([msg]);
    expect(out).toContain('[tool_call] shell::run');
    expect(out).toContain('command');
  });
});
