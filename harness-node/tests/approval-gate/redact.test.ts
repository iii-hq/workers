import { describe, expect, it } from 'vitest';
import { clip, redact } from '../../src/approval-gate/redact.js';

describe('redact (immutable, recursive)', () => {
  it('passes through null / number / boolean unchanged', () => {
    expect(redact(null)).toBeNull();
    expect(redact(undefined)).toBeUndefined();
    expect(redact(42)).toBe(42);
    expect(redact(true)).toBe(true);
  });

  it('clips long strings to 256 code points', () => {
    const long = 'a'.repeat(300);
    const out = redact(long);
    expect(typeof out).toBe('string');
    expect([...(out as string)].length).toBe(257); // 256 chars + ellipsis
    expect(out).toMatch(/…$/);
  });

  it('redacts a top-level secret key (case-insensitive)', () => {
    const out = redact({ Password: 'hunter2', name: 'alice' }) as Record<string, unknown>;
    expect(out.Password).toBe('<redacted>');
    expect(out.name).toBe('alice');
  });

  it('redacts a suffix-matched secret key (foo_password)', () => {
    const out = redact({ db_password: 's3cret', dbHost: 'x' }) as Record<string, unknown>;
    expect(out.db_password).toBe('<redacted>');
    expect(out.dbHost).toBe('x');
  });

  it('does NOT redact unrelated keys that merely contain secret substrings', () => {
    const out = redact({ tokenizer: 'tiktoken', authority: 'system' }) as Record<string, unknown>;
    expect(out.tokenizer).toBe('tiktoken');
    expect(out.authority).toBe('system');
  });

  it('redacts deep inside nested objects and arrays', () => {
    const out = redact({
      config: { credentials: { api_key: 'k', region: 'us' } },
      hosts: [{ token: 't', name: 'h1' }],
    }) as Record<string, unknown>;
    const config = out.config as Record<string, unknown>;
    const creds = config.credentials as Record<string, unknown>;
    expect(creds.api_key).toBe('<redacted>');
    expect(creds.region).toBe('us');
    const hosts = out.hosts as Array<Record<string, unknown>>;
    expect(hosts[0].token).toBe('<redacted>');
    expect(hosts[0].name).toBe('h1');
  });

  it('handles Unicode strings without breaking surrogate pairs', () => {
    const emojis = '🎉'.repeat(300);
    const out = redact(emojis) as string;
    expect([...out].length).toBe(257);
    expect(out.endsWith('…')).toBe(true);
  });

  // R2 — immutability regression guard. Freezing the input would throw in
  // strict mode if redact tried to assign to it; we additionally snapshot
  // before/after to catch any future logic that mutates structurally.
  it('does NOT mutate the input (R2 regression guard)', () => {
    const input = {
      password: 'hunter2',
      nested: { token: 't', list: ['a', 'b'] },
    };
    const snapshot = JSON.parse(JSON.stringify(input));
    Object.freeze(input);
    Object.freeze(input.nested);
    Object.freeze(input.nested.list);

    const out = redact(input) as Record<string, unknown>;
    expect(out.password).toBe('<redacted>');

    // Input must be structurally identical to its snapshot.
    expect(input).toEqual(snapshot);
    // Output must be a different object (new tree).
    expect(out).not.toBe(input);
    expect(out.nested).not.toBe(input.nested);
  });
});

describe('clip', () => {
  it('returns short strings unchanged', () => {
    expect(clip('hello')).toBe('hello');
  });
  it('truncates and appends ellipsis when longer than cap', () => {
    const out = clip('x'.repeat(300));
    expect(out.length).toBe(257);
    expect(out.endsWith('…')).toBe(true);
  });
});
