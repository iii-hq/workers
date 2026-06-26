import { describe, expect, it } from 'vitest';
import { loadConfig } from '../src/config.js';
import { buildArgs, extractPrompt, RunPayloadSchema } from '../src/run.js';

const cfg = await loadConfig('/nonexistent/config.yaml');

describe('RunPayloadSchema', () => {
  it('accepts a bare prompt', () => {
    expect(RunPayloadSchema.parse({ prompt: 'hi' }).prompt).toBe('hi');
  });
  it('accepts the messages array shape', () => {
    const p = RunPayloadSchema.parse({
      session_id: 's1',
      messages: [{ role: 'user', content: [{ type: 'text', text: 'hello' }] }],
    });
    expect(p.messages).toHaveLength(1);
  });
  it('rejects a negative timeout', () => {
    expect(() => RunPayloadSchema.parse({ prompt: 'x', timeout_ms: -1 })).toThrow();
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
            { type: 'text', text: 'a' },
            { type: 'text', text: 'b' },
          ],
        },
      ],
    });
    expect(extractPrompt(p)).toBe('a\nb');
  });
  it('accepts plain-string content', () => {
    expect(
      extractPrompt(RunPayloadSchema.parse({ messages: [{ role: 'user', content: 'plain' }] })),
    ).toBe('plain');
  });
  it('throws when no user message exists', () => {
    expect(() =>
      extractPrompt(RunPayloadSchema.parse({ messages: [{ role: 'assistant', content: 'x' }] })),
    ).toThrow();
  });
});

describe('buildArgs', () => {
  it('builds run --format json with the prompt last', () => {
    const args = buildArgs(RunPayloadSchema.parse({ prompt: 'hi' }), cfg, 'hi', null);
    expect(args.slice(0, 3)).toEqual(['run', '--format', 'json']);
    expect(args[args.length - 1]).toBe('hi');
    expect(args).not.toContain('--session');
  });
  it('adds --session on resume and --model/--dir when set', () => {
    const args = buildArgs(
      RunPayloadSchema.parse({ prompt: 'hi', model: 'anthropic/claude-sonnet-4-5', cwd: '/repo' }),
      cfg,
      'hi',
      'ses_prior',
    );
    expect(args).toContain('--session');
    expect(args[args.indexOf('--session') + 1]).toBe('ses_prior');
    expect(args[args.indexOf('--model') + 1]).toBe('anthropic/claude-sonnet-4-5');
    expect(args[args.indexOf('--dir') + 1]).toBe('/repo');
  });

  it('adds --agent when set, omits flags that are empty', () => {
    const args = buildArgs(
      RunPayloadSchema.parse({ prompt: 'hi', agent: 'build' }),
      cfg,
      'hi',
      null,
    );
    expect(args[args.indexOf('--agent') + 1]).toBe('build');
    expect(args).not.toContain('--model');
    expect(args).not.toContain('--dir');
  });

  it('per-turn fields override config defaults', async () => {
    const c2 = await loadConfig('/nonexistent/config.yaml');
    c2.defaults.model = 'cfg/model';
    const args = buildArgs(
      RunPayloadSchema.parse({ prompt: 'hi', model: 'payload/model' }),
      c2,
      'hi',
      null,
    );
    expect(args[args.indexOf('--model') + 1]).toBe('payload/model');
  });
});
