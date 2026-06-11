import { describe, expect, it } from 'vitest';
import { extractPrompt, RunPayloadSchema } from '../src/run.js';

describe('RunPayloadSchema', () => {
  it('accepts a bare prompt', () => {
    const p = RunPayloadSchema.parse({ prompt: 'hi' });
    expect(p.prompt).toBe('hi');
  });

  it('accepts the messages array shape', () => {
    const p = RunPayloadSchema.parse({
      session_id: 's1',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
    });
    expect(p.messages).toHaveLength(1);
  });

  it('rejects an invalid sandbox mode', () => {
    expect(() => RunPayloadSchema.parse({ prompt: 'x', sandbox_mode: 'yolo' })).toThrow();
  });
});

describe('extractPrompt', () => {
  it('prefers the prompt field', () => {
    expect(extractPrompt(RunPayloadSchema.parse({ prompt: 'direct' }))).toBe('direct');
  });

  it('joins text blocks from the last user message', () => {
    const p = RunPayloadSchema.parse({
      messages: [
        { role: 'user', content: [{ type: 'text', text: 'first' }] },
        { role: 'assistant', content: [{ type: 'text', text: 'reply' }] },
        {
          role: 'user',
          content: [
            { type: 'text', text: 'line one' },
            { type: 'text', text: 'line two' },
          ],
        },
      ],
    });
    expect(extractPrompt(p)).toBe('line one\nline two');
  });

  it('accepts plain-string message content', () => {
    const p = RunPayloadSchema.parse({ messages: [{ role: 'user', content: 'plain' }] });
    expect(extractPrompt(p)).toBe('plain');
  });

  it('throws when no user message exists', () => {
    const p = RunPayloadSchema.parse({ messages: [{ role: 'assistant', content: 'x' }] });
    expect(() => extractPrompt(p)).toThrow();
  });
});
