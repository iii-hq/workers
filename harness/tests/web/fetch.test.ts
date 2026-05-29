import { Buffer } from 'node:buffer';
import { Readable } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { loadWebConfig } from '../../src/web/config.js';
import { executeFetch, readIncomingCapped, stripCrossOriginAuth } from '../../src/web/fetch.js';
import { FetchPayloadSchema } from '../../src/web/schemas.js';

// The SSRF guard is exhaustively tested in ssrf.test.ts. These tests
// focus on:
//   1. The error surface of executeFetch (invalid url, blocked host).
//   2. The body-cap helper that bounds response size.
//   3. Config loading defaults / overrides.
// Live HTTP integration (timeouts, redirect chains hitting public
// hosts) is left to the manual smoke step in the plan's verification
// section — going out over the real network from unit tests is flaky
// and the security-critical paths are already covered.

describe('executeFetch payload + guard surface', () => {
  it('returns invalid_url for a non-http scheme', async () => {
    const cfg = loadWebConfig({});
    const r = await executeFetch({ url: 'file:///etc/passwd' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('invalid_url');
  });

  it('returns invalid_url for garbage input', async () => {
    const cfg = loadWebConfig({});
    const r = await executeFetch({ url: 'not a url' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('invalid_url');
  });

  it('returns blocked_host for AWS metadata IP literal', async () => {
    const cfg = loadWebConfig({});
    const r = await executeFetch({ url: 'http://169.254.169.254/latest/' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('blocked_host');
  });

  it('allows loopback when allow_loopback is true (default for harness UX)', async () => {
    const cfg = loadWebConfig({});
    // 127.0.0.1:1 is a closed port — we expect the SSRF guard to PASS
    // and the request to fail at the transport layer (connection refused).
    const r = await executeFetch({ url: 'http://127.0.0.1:1/' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('transport_error');
  });

  it('blocks loopback when allow_loopback is explicitly disabled', async () => {
    const cfg = loadWebConfig({ web: { allow_loopback: false } });
    const r = await executeFetch({ url: 'http://127.0.0.1:8080/secret' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('blocked_host');
  });

  it('returns blocked_host for a private RFC1918 address', async () => {
    const cfg = loadWebConfig({});
    const r = await executeFetch({ url: 'http://192.168.1.1/' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('blocked_host');
  });

  it('strict-mode blocked_host on loopback gives a config hint', async () => {
    const cfg = loadWebConfig({ web: { allow_loopback: false } });
    const r = await executeFetch({ url: 'http://127.0.0.1:8080/' }, cfg);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toBe('blocked_host');
      expect(r.message).toMatch(/web\.allow_loopback=true/);
    }
  });
});

describe('schema preprocessing — case-insensitive method + json field', () => {
  it('normalises lowercase methods to upper', () => {
    const r = FetchPayloadSchema.safeParse({ url: 'http://x/', method: 'post' });
    expect(r.success).toBe(true);
    if (r.success) expect(r.data.method).toBe('POST');
  });

  it('normalises mixed-case methods', () => {
    const r = FetchPayloadSchema.safeParse({ url: 'http://x/', method: 'Get' });
    expect(r.success).toBe(true);
    if (r.success) expect(r.data.method).toBe('GET');
  });

  it('rejects nonsense methods', () => {
    const r = FetchPayloadSchema.safeParse({ url: 'http://x/', method: 'BREW' });
    expect(r.success).toBe(false);
  });

  it('accepts an unknown json payload of any shape', () => {
    const r1 = FetchPayloadSchema.safeParse({ url: 'http://x/', json: { a: 1, b: [2, 3] } });
    const r2 = FetchPayloadSchema.safeParse({ url: 'http://x/', json: 'a string is valid json' });
    const r3 = FetchPayloadSchema.safeParse({ url: 'http://x/', json: null });
    expect(r1.success && r2.success && r3.success).toBe(true);
  });

  it('accepts response_format: "json"', () => {
    const r = FetchPayloadSchema.safeParse({ url: 'http://x/', response_format: 'json' });
    expect(r.success).toBe(true);
  });
});

describe('stripCrossOriginAuth', () => {
  const AUTH = { authorization: 'Bearer secret', cookie: 'sid=abc', accept: 'text/html' };

  it('keeps auth on a same-origin redirect', () => {
    const out = stripCrossOriginAuth(
      AUTH,
      new URL('https://api.example.com/a'),
      new URL('https://api.example.com/b'),
    );
    expect(out.authorization).toBe('Bearer secret');
    expect(out.cookie).toBe('sid=abc');
  });

  it('strips auth on a cross-host redirect', () => {
    const out = stripCrossOriginAuth(
      AUTH,
      new URL('https://api.example.com/a'),
      new URL('https://evil.example.net/b'),
    );
    expect(out.authorization).toBeUndefined();
    expect(out.cookie).toBeUndefined();
    expect(out.accept).toBe('text/html'); // non-sensitive header survives
  });

  it('strips auth on a same-host https→http downgrade (no cleartext leak)', () => {
    const out = stripCrossOriginAuth(
      AUTH,
      new URL('https://api.example.com/a'),
      new URL('http://api.example.com/b'),
    );
    expect(out.authorization).toBeUndefined();
    expect(out.cookie).toBeUndefined();
  });

  it('keeps auth on a same-host http→https upgrade', () => {
    const out = stripCrossOriginAuth(
      AUTH,
      new URL('http://api.example.com/a'),
      new URL('https://api.example.com/b'),
    );
    expect(out.authorization).toBe('Bearer secret');
  });
});

describe('readIncomingCapped', () => {
  // Multi-chunk stream so the cap is exercised across chunk boundaries, the
  // way a real IncomingMessage delivers a body.
  function makeStream(bytes: Buffer, chunkSize = 4096): Readable {
    const chunks: Buffer[] = [];
    for (let i = 0; i < bytes.length; i += chunkSize) {
      chunks.push(bytes.subarray(i, i + chunkSize));
    }
    return Readable.from(chunks.length > 0 ? chunks : []);
  }

  it('reads full body when under cap', async () => {
    const r = await readIncomingCapped(makeStream(Buffer.from('hello world')), 1024);
    expect(r.truncated).toBe(false);
    expect(r.bytes.toString('utf8')).toBe('hello world');
  });

  it('truncates body at exactly the cap (across chunks)', async () => {
    const r = await readIncomingCapped(makeStream(Buffer.alloc(10_000, 0x41)), 100);
    expect(r.truncated).toBe(true);
    expect(r.bytes.length).toBe(100);
  });

  it('handles empty body', async () => {
    const r = await readIncomingCapped(makeStream(Buffer.alloc(0)), 1024);
    expect(r.truncated).toBe(false);
    expect(r.bytes.length).toBe(0);
  });

  it('honours a tiny cap', async () => {
    const r = await readIncomingCapped(makeStream(Buffer.from('aaaaaaaaaa')), 1);
    expect(r.truncated).toBe(true);
    expect(r.bytes.length).toBe(1);
  });
});

describe('loadWebConfig', () => {
  it('produces sane defaults from empty config', () => {
    const cfg = loadWebConfig({});
    expect(cfg.max_timeout_ms).toBe(30_000);
    expect(cfg.max_response_bytes).toBe(5 * 1024 * 1024);
    expect(cfg.max_redirects).toBe(5);
    expect(cfg.user_agent).toMatch(/iii-harness/);
    expect(cfg.allow_loopback).toBe(true);
  });

  it('honours overrides under web: section', () => {
    const cfg = loadWebConfig({
      web: {
        max_timeout_ms: 5000,
        max_response_bytes: 1024,
        max_redirects: 1,
        user_agent: 'custom',
        allow_loopback: false,
      },
    });
    expect(cfg.max_timeout_ms).toBe(5000);
    expect(cfg.max_response_bytes).toBe(1024);
    expect(cfg.max_redirects).toBe(1);
    expect(cfg.user_agent).toBe('custom');
    expect(cfg.allow_loopback).toBe(false);
  });

  it('ignores non-numeric override values', () => {
    const cfg = loadWebConfig({ web: { max_timeout_ms: 'fast' } });
    expect(cfg.max_timeout_ms).toBe(30_000);
  });
});
