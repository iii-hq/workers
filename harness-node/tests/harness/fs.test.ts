import { describe, expect, it } from 'vitest';
import { buildInlineEnvelope } from '../../src/harness/fs.js';

describe('buildInlineEnvelope', () => {
  it('round-trips small UTF-8', () => {
    const env = buildInlineEnvelope(new TextEncoder().encode('hello'), 5) as Record<
      string,
      unknown
    >;
    const content = env.content as Array<{ type: string; text: string }>;
    expect(content[0]?.text).toBe('hello');
    const details = env.details as Record<string, unknown>;
    expect(details.size).toBe(5);
    expect(details.truncated).toBe(false);
    expect(env.terminate).toBe(false);
  });

  it('marks truncation when drained < total', () => {
    const env = buildInlineEnvelope(new Uint8Array(256).fill('x'.charCodeAt(0)), 1000) as Record<
      string,
      unknown
    >;
    const details = env.details as Record<string, unknown>;
    expect(details.size).toBe(1000);
    expect(details.truncated).toBe(true);
    expect(details.bytes_read).toBe(256);
  });

  it('emits binary marker on invalid UTF-8', () => {
    const env = buildInlineEnvelope(new Uint8Array([0xff, 0xfe, 0xfd]), 3) as Record<
      string,
      unknown
    >;
    const content = env.content as Array<{ type: string; text: string }>;
    expect(content[0]?.text).toBe('<binary 3 bytes>');
  });
});
