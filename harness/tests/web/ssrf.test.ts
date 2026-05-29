import { describe, expect, it } from 'vitest';
import { __testing, checkTarget, parseTarget } from '../../src/web/ssrf.js';

const { isBlockedIpv4, isBlockedIpv6 } = __testing;

const STRICT = { allow_loopback: false };
const LAX = { allow_loopback: true };

describe('parseTarget', () => {
  it('accepts http and https', () => {
    expect(parseTarget('http://example.com/')).toMatchObject({ hostname: 'example.com', port: 80 });
    expect(parseTarget('https://example.com/')).toMatchObject({ port: 443 });
  });

  it('honours explicit ports', () => {
    const r = parseTarget('http://example.com:8080/x');
    expect(r).toMatchObject({ port: 8080 });
  });

  it('rejects non-http schemes', () => {
    expect(parseTarget('file:///etc/passwd')).toEqual({
      error: expect.stringMatching(/scheme not allowed/),
    });
    expect(parseTarget('ftp://example.com/')).toEqual({
      error: expect.stringMatching(/scheme not allowed/),
    });
    expect(parseTarget('javascript:alert(1)')).toEqual({
      error: expect.stringMatching(/scheme not allowed/),
    });
    expect(parseTarget('data:text/plain,hi')).toEqual({
      error: expect.stringMatching(/scheme not allowed/),
    });
  });

  it('rejects garbage urls', () => {
    expect(parseTarget('not a url')).toHaveProperty('error');
    expect(parseTarget('http://')).toHaveProperty('error');
  });
});

describe('isBlockedIpv4', () => {
  it('blocks loopback variants under strict policy', () => {
    expect(isBlockedIpv4('127.0.0.1', STRICT).blocked).toBe(true);
    expect(isBlockedIpv4('127.255.255.254', STRICT).blocked).toBe(true);
  });

  it('allows loopback under default (lax) policy — harness UX', () => {
    expect(isBlockedIpv4('127.0.0.1', LAX).blocked).toBe(false);
    expect(isBlockedIpv4('127.0.0.1').blocked).toBe(false); // default arg
  });

  it('blocks AWS / cloud metadata', () => {
    const v = isBlockedIpv4('169.254.169.254');
    expect(v.blocked).toBe(true);
    expect(v.label).toMatch(/link-local/);
  });

  it('blocks all RFC1918 ranges', () => {
    expect(isBlockedIpv4('10.0.0.1').blocked).toBe(true);
    expect(isBlockedIpv4('10.255.255.255').blocked).toBe(true);
    expect(isBlockedIpv4('172.16.0.1').blocked).toBe(true);
    expect(isBlockedIpv4('172.31.255.255').blocked).toBe(true);
    expect(isBlockedIpv4('192.168.0.1').blocked).toBe(true);
    expect(isBlockedIpv4('192.168.255.255').blocked).toBe(true);
  });

  it('does NOT block public addresses', () => {
    expect(isBlockedIpv4('8.8.8.8').blocked).toBe(false);
    expect(isBlockedIpv4('1.1.1.1').blocked).toBe(false);
    expect(isBlockedIpv4('140.82.114.4').blocked).toBe(false); // github.com sample
  });

  it('blocks 0.0.0.0', () => {
    expect(isBlockedIpv4('0.0.0.0').blocked).toBe(true);
  });

  it('blocks 172.31 (top of rfc1918 172/12) but allows 172.32 (outside)', () => {
    expect(isBlockedIpv4('172.31.0.1').blocked).toBe(true);
    expect(isBlockedIpv4('172.32.0.1').blocked).toBe(false);
  });
});

describe('isBlockedIpv6', () => {
  it('blocks loopback ::1 under strict policy', () => {
    expect(isBlockedIpv6('::1', STRICT).blocked).toBe(true);
  });

  it('allows loopback ::1 under default (lax) policy', () => {
    expect(isBlockedIpv6('::1', LAX).blocked).toBe(false);
    expect(isBlockedIpv6('::1').blocked).toBe(false);
  });

  it('blocks unspecified ::', () => {
    expect(isBlockedIpv6('::').blocked).toBe(true);
  });

  it('blocks link-local fe80::/10', () => {
    expect(isBlockedIpv6('fe80::1').blocked).toBe(true);
  });

  it('blocks unique-local fc00::/7', () => {
    expect(isBlockedIpv6('fc00::1').blocked).toBe(true);
    expect(isBlockedIpv6('fd00::1').blocked).toBe(true);
  });

  it('blocks multicast', () => {
    expect(isBlockedIpv6('ff02::1').blocked).toBe(true);
  });

  it('blocks IPv4-mapped private ranges (loopback only under strict policy)', () => {
    expect(isBlockedIpv6('::ffff:127.0.0.1', STRICT).blocked).toBe(true);
    expect(isBlockedIpv6('::ffff:127.0.0.1', LAX).blocked).toBe(false);
    // Metadata IP is blocked regardless of allow_loopback.
    expect(isBlockedIpv6('::ffff:169.254.169.254', STRICT).blocked).toBe(true);
    expect(isBlockedIpv6('::ffff:169.254.169.254', LAX).blocked).toBe(true);
  });

  it('blocks IPv4-mapped private ranges in HEX form (regression: metadata bypass)', () => {
    // ::ffff:a9fe:a9fe IS 169.254.169.254 — the dotted-only decode let this
    // hex form slip past the guard and reach cloud metadata.
    expect(isBlockedIpv6('::ffff:a9fe:a9fe').blocked).toBe(true);
    expect(isBlockedIpv6('::ffff:A9FE:A9FE').blocked).toBe(true); // case-insensitive
    // 10.0.0.1 = ::ffff:0a00:0001 (rfc1918, blocked under any policy)
    expect(isBlockedIpv6('::ffff:0a00:0001', LAX).blocked).toBe(true);
    // 127.0.0.1 = ::ffff:7f00:0001 (loopback — lax allows, strict blocks)
    expect(isBlockedIpv6('::ffff:7f00:0001', STRICT).blocked).toBe(true);
    expect(isBlockedIpv6('::ffff:7f00:0001', LAX).blocked).toBe(false);
    // A public address in hex stays allowed (8.8.8.8 = ::ffff:0808:0808).
    expect(isBlockedIpv6('::ffff:0808:0808').blocked).toBe(false);
  });

  it('does NOT block public IPv6', () => {
    expect(isBlockedIpv6('2606:4700::1111').blocked).toBe(false); // cloudflare
    expect(isBlockedIpv6('2001:4860:4860::8888').blocked).toBe(false); // google
  });
});

describe('checkTarget (literal IPs, no DNS)', () => {
  it('passes a public ipv4 literal', async () => {
    const p = parseTarget('http://8.8.8.8/');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p);
    expect(r.ok).toBe(true);
  });

  it('blocks AWS metadata IP literal', async () => {
    const p = parseTarget('http://169.254.169.254/latest/meta-data/');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toBe('blocked_host');
      expect(r.message).toMatch(/169\.254\.169\.254/);
    }
  });

  it('allows loopback ipv4 literal by default (harness UX)', async () => {
    const p = parseTarget('http://127.0.0.1:8080/admin');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p);
    expect(r.ok).toBe(true);
  });

  it('blocks loopback ipv4 literal under strict policy', async () => {
    const p = parseTarget('http://127.0.0.1:8080/admin');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p, STRICT);
    expect(r.ok).toBe(false);
  });

  it('allows bracketed ipv6 loopback by default', async () => {
    const p = parseTarget('http://[::1]:8080/');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p);
    expect(r.ok).toBe(true);
  });

  it('blocks bracketed ipv6 loopback under strict policy', async () => {
    const p = parseTarget('http://[::1]:8080/');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p, STRICT);
    expect(r.ok).toBe(false);
  });

  it('allows DNS that resolves to localhost by default (the bug we fixed)', async () => {
    const p = parseTarget('http://localhost/');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p);
    expect(r.ok).toBe(true);
  });

  it('blocks DNS that resolves to localhost under strict policy', async () => {
    const p = parseTarget('http://localhost/');
    if ('error' in p) throw new Error(p.error);
    const r = await checkTarget(p, STRICT);
    expect(r.ok).toBe(false);
  });
});
