import { Buffer } from 'node:buffer';
import { Readable } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { loadWebConfig } from '../../src/web/config.js';
import {
  executeFetch,
  readIncomingCapped,
  resolveMaxBytes,
  resolveTimeout,
  stripCrossOriginAuth,
} from '../../src/web/fetch.js';
import {
  type FetchImageResult,
  FetchPayloadSchema,
  type FetchResult,
} from '../../src/web/schemas.js';

// None of the payloads in this file set `format`, so executeFetch always
// returns the plain FetchResult envelope — narrow the union once here.
function asFetchResult(r: FetchResult | FetchImageResult): FetchResult {
  if ('content' in r) throw new Error('unexpected image envelope');
  return r;
}

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
    const r = asFetchResult(await executeFetch({ url: 'file:///etc/passwd' }, cfg));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('invalid_url');
  });

  it('returns invalid_url for garbage input', async () => {
    const cfg = loadWebConfig({});
    const r = asFetchResult(await executeFetch({ url: 'not a url' }, cfg));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('invalid_url');
  });

  it('returns blocked_host for AWS metadata IP literal', async () => {
    const cfg = loadWebConfig({});
    const r = asFetchResult(await executeFetch({ url: 'http://169.254.169.254/latest/' }, cfg));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('blocked_host');
  });

  it('allows loopback when allow_loopback is true (default for harness UX)', async () => {
    const cfg = loadWebConfig({});
    // 127.0.0.1:1 is a closed port — we expect the SSRF guard to PASS
    // and the request to fail at the transport layer (connection refused).
    const r = asFetchResult(await executeFetch({ url: 'http://127.0.0.1:1/' }, cfg));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('transport_error');
  });

  it('blocks loopback when allow_loopback is explicitly disabled', async () => {
    const cfg = loadWebConfig({ web: { allow_loopback: false } });
    const r = asFetchResult(await executeFetch({ url: 'http://127.0.0.1:8080/secret' }, cfg));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('blocked_host');
  });

  it('returns blocked_host for a private RFC1918 address', async () => {
    const cfg = loadWebConfig({});
    const r = asFetchResult(await executeFetch({ url: 'http://192.168.1.1/' }, cfg));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('blocked_host');
  });

  it('strict-mode blocked_host on loopback gives a config hint', async () => {
    const cfg = loadWebConfig({ web: { allow_loopback: false } });
    const r = asFetchResult(await executeFetch({ url: 'http://127.0.0.1:8080/' }, cfg));
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

  it('accepts every page format value', () => {
    for (const format of ['markdown', 'text', 'html']) {
      const r = FetchPayloadSchema.safeParse({ url: 'http://x/', format });
      expect(r.success).toBe(true);
    }
  });

  it('rejects an unknown page format', () => {
    const r = FetchPayloadSchema.safeParse({ url: 'http://x/', format: 'pdf' });
    expect(r.success).toBe(false);
  });

  it('still parses with format absent (backward compat)', () => {
    const r = FetchPayloadSchema.safeParse({ url: 'http://x/' });
    expect(r.success).toBe(true);
    if (r.success) expect(r.data.format).toBeUndefined();
  });
});

describe('resolveTimeout', () => {
  it('defaults to default_timeout_ms when the caller passes nothing', () => {
    const cfg = loadWebConfig({});
    expect(resolveTimeout({ url: 'http://x/' }, cfg)).toBe(30_000);
  });

  it('honours a caller timeout below the ceiling', () => {
    const cfg = loadWebConfig({});
    expect(resolveTimeout({ url: 'http://x/', timeout_ms: 60_000 }, cfg)).toBe(60_000);
  });

  it('clamps a caller timeout above max_timeout_ms down to the ceiling', () => {
    const cfg = loadWebConfig({});
    expect(resolveTimeout({ url: 'http://x/', timeout_ms: 999_999 }, cfg)).toBe(120_000);
  });

  it('clamps the default down when an operator sets a ceiling below it', () => {
    const cfg = loadWebConfig({ web: { max_timeout_ms: 5_000 } });
    expect(resolveTimeout({ url: 'http://x/' }, cfg)).toBe(5_000);
  });
});

describe('resolveMaxBytes', () => {
  // Regression: a raw fetch (no `format`) that omits max_bytes must keep
  // defaulting to the 5 MiB ceiling, not the context-safe page-mode cap.
  // The smaller default silently truncated existing API/download callers
  // and handed back partial bodies as if complete.
  it('defaults a raw fetch to the hard ceiling, not the page-mode cap', () => {
    const cfg = loadWebConfig({});
    expect(resolveMaxBytes({ url: 'http://x/' }, cfg)).toBe(5 * 1024 * 1024);
  });

  it('defaults page-reading mode to the context-safe cap', () => {
    const cfg = loadWebConfig({});
    expect(resolveMaxBytes({ url: 'http://x/', format: 'markdown' }, cfg)).toBe(256 * 1024);
  });

  it('honours an explicit max_bytes below the ceiling in both modes', () => {
    const cfg = loadWebConfig({});
    expect(resolveMaxBytes({ url: 'http://x/', max_bytes: 1024 }, cfg)).toBe(1024);
    expect(resolveMaxBytes({ url: 'http://x/', format: 'markdown', max_bytes: 1024 }, cfg)).toBe(
      1024,
    );
  });

  it('clamps an explicit max_bytes above the ceiling down to max_response_bytes', () => {
    const cfg = loadWebConfig({});
    expect(resolveMaxBytes({ url: 'http://x/', max_bytes: 50 * 1024 * 1024 }, cfg)).toBe(
      5 * 1024 * 1024,
    );
  });

  it('clamps the page-mode default down when an operator sets a ceiling below it', () => {
    const cfg = loadWebConfig({ web: { max_response_bytes: 1024 } });
    expect(resolveMaxBytes({ url: 'http://x/', format: 'markdown' }, cfg)).toBe(1024);
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
    expect(cfg.default_timeout_ms).toBe(30_000);
    expect(cfg.max_timeout_ms).toBe(120_000);
    expect(cfg.default_response_bytes).toBe(256 * 1024);
    expect(cfg.max_response_bytes).toBe(5 * 1024 * 1024);
    expect(cfg.max_transform_bytes).toBe(1024 * 1024);
    expect(cfg.max_redirects).toBe(5);
    expect(cfg.user_agent).toMatch(/iii-harness/);
    expect(cfg.allow_loopback).toBe(true);
  });

  it('honours overrides under web: section', () => {
    const cfg = loadWebConfig({
      web: {
        default_timeout_ms: 2000,
        max_timeout_ms: 5000,
        max_response_bytes: 1024,
        max_redirects: 1,
        user_agent: 'custom',
        allow_loopback: false,
      },
    });
    expect(cfg.default_timeout_ms).toBe(2000);
    expect(cfg.max_timeout_ms).toBe(5000);
    expect(cfg.max_response_bytes).toBe(1024);
    expect(cfg.max_redirects).toBe(1);
    expect(cfg.user_agent).toBe('custom');
    expect(cfg.allow_loopback).toBe(false);
  });

  it('ignores non-numeric override values', () => {
    const cfg = loadWebConfig({ web: { max_timeout_ms: 'fast' } });
    expect(cfg.max_timeout_ms).toBe(120_000);
  });
});
