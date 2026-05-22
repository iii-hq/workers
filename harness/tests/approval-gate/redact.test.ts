/**
 * Adversarial tests for the secret-redaction tree. This is the worker's
 * last line of defence against leaking credentials into denial envelopes,
 * so every test here is an attempt to (a) sneak a secret past the filter,
 * (b) crash the redactor with hostile input, or (c) corrupt the process.
 */
import { describe, expect, it } from 'vitest';
import { clip, redact } from '../../src/approval-gate/redact.js';

describe('clip — code-point cap (256)', () => {
  it('leaves a string exactly at the cap untouched (no false ellipsis)', () => {
    const at = 'a'.repeat(256);
    expect(clip(at)).toBe(at);
  });

  it('clips one past the cap to 256 code points + ellipsis', () => {
    const out = clip('a'.repeat(257));
    expect([...out].length).toBe(257); // 256 kept + '…'
    expect(out.endsWith('…')).toBe(true);
  });

  it('does NOT clip an astral string whose code-point length is within the cap', () => {
    // 200 emoji = 200 code points (under cap) but 400 UTF-16 units (over it).
    // A UTF-16-length guard would falsely truncate + ellipsize this.
    const s = '🎉'.repeat(200);
    expect(clip(s)).toBe(s);
  });

  it('clips an astral string over the cap without splitting surrogate pairs', () => {
    const out = clip('🎉'.repeat(300));
    expect([...out].length).toBe(257);
    expect(out.endsWith('…')).toBe(true);
    // Every retained unit is a whole emoji — no lone surrogate (�/half pair).
    expect([...out].slice(0, 256).every((c) => c === '🎉')).toBe(true);
  });
});

describe('redact — secret key matching', () => {
  it('redacts exact and suffix-matched secret keys, case-insensitively', () => {
    const out = redact({
      Password: 'hunter2',
      db_password: 's3cret',
      api_key: 'k',
      TOKEN: 't',
    }) as Record<string, unknown>;
    expect(out.Password).toBe('<redacted>');
    expect(out.db_password).toBe('<redacted>');
    expect(out.api_key).toBe('<redacted>');
    expect(out.TOKEN).toBe('<redacted>');
  });

  it('does not over-match keys that merely contain a secret substring', () => {
    const out = redact({
      tokenizer: 'tiktoken',
      authority: 'system',
      secretary: 'alice',
      passwordless: true,
    }) as Record<string, unknown>;
    expect(out.tokenizer).toBe('tiktoken');
    expect(out.authority).toBe('system');
    expect(out.secretary).toBe('alice');
    expect(out.passwordless).toBe(true);
  });

  it('redacts a secret-keyed value wholesale, even when it is a nested object', () => {
    // Attack: hide a secret one level under a secret key, hoping only the
    // top string gets blanked. The whole subtree must collapse to <redacted>.
    const out = redact({
      authorization: { scheme: 'Bearer', value: 'xyz' },
    }) as Record<string, unknown>;
    expect(out.authorization).toBe('<redacted>');
  });

  it('redacts secrets buried deep in nested objects and arrays', () => {
    const out = redact({
      config: { credentials: { access_token: 'k', region: 'us' } },
      hosts: [{ refresh_token: 't', name: 'h1' }],
    }) as Record<string, unknown>;
    const creds = (out.config as Record<string, Record<string, unknown>>).credentials;
    expect(creds.access_token).toBe('<redacted>');
    expect(creds.region).toBe('us');
    const hosts = out.hosts as Array<Record<string, unknown>>;
    expect(hosts[0].refresh_token).toBe('<redacted>');
    expect(hosts[0].name).toBe('h1');
  });
});

describe('redact — known boundaries (documented gaps, not bugs)', () => {
  it('is key-based only: a secret hidden in a non-secret value is NOT scrubbed', () => {
    // Documented limitation — redaction keys off field names, not contents.
    const out = redact({ note: 'the password is hunter2' }) as Record<string, unknown>;
    expect(out.note).toBe('the password is hunter2');
  });

  it('drops symbol-keyed entries (Object.entries ignores them)', () => {
    const out = redact({ [Symbol('password')]: 'leak', keep: 1 }) as Record<string, unknown>;
    expect(out).toEqual({ keep: 1 });
  });
});

describe('redact — hostile structure must not crash the worker', () => {
  it('survives pathologically deep nesting without a stack overflow', () => {
    let node: Record<string, unknown> = {};
    const root = node;
    for (let i = 0; i < 200_000; i++) {
      const next: Record<string, unknown> = {};
      node.n = next;
      node = next;
    }
    expect(() => redact(root)).not.toThrow();
  });

  it('survives a circular reference without infinite recursion', () => {
    const a: Record<string, unknown> = { password: 'p' };
    a.self = a;
    let out: Record<string, unknown> = {};
    expect(() => {
      out = redact(a) as Record<string, unknown>;
    }).not.toThrow();
    expect(out.password).toBe('<redacted>');
  });
});

describe('redact — process integrity', () => {
  it('does not pollute Object.prototype via a __proto__ key', () => {
    // JSON.parse produces an own enumerable "__proto__" data property — the
    // classic prototype-pollution vector. redact must keep it a plain key.
    const payload = JSON.parse('{"__proto__": {"password": "x"}, "a": 1}');
    const out = redact(payload) as Record<string, unknown>;
    expect(({} as Record<string, unknown>).password).toBeUndefined();
    expect(Object.keys(out)).toContain('__proto__');
  });

  it('never mutates its input (frozen tree round-trips intact)', () => {
    const input = { password: 'hunter2', nested: { token: 't', list: ['a', 'b'] } };
    const snapshot = JSON.parse(JSON.stringify(input));
    Object.freeze(input);
    Object.freeze(input.nested);
    Object.freeze(input.nested.list);

    const out = redact(input) as Record<string, unknown>;
    expect(out.password).toBe('<redacted>');
    expect(input).toEqual(snapshot);
    expect(out).not.toBe(input);
    expect(out.nested).not.toBe(input.nested);
  });

  it('passes primitives through unchanged', () => {
    expect(redact(null)).toBeNull();
    expect(redact(undefined)).toBeUndefined();
    expect(redact(42)).toBe(42);
    expect(redact(true)).toBe(true);
  });
});
