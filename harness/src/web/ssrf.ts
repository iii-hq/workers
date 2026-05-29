/**
 * SSRF defense for `web::fetch`. Parses the target URL, resolves the
 * host to its concrete addresses, and rejects any address that lands
 * in a private, loopback, link-local, multicast, or reserved range.
 *
 * The resolve is done ONCE here; the actual `fetch()` is then called
 * with the resolved IP literal so undici cannot re-resolve and land
 * on a different (private) address — defending against DNS rebinding.
 * TLS / vhost routing is preserved by overriding the `Host` header to
 * the original hostname.
 */

import { isIPv4, isIPv6 } from 'node:net';
import { lookup as dnsLookup } from 'node:dns/promises';

export type SsrfPolicy = {
  /** When true, 127.0.0.0/8 and ::1 are NOT blocked. Other private
   * ranges (RFC1918, link-local, ULA, multicast, AWS metadata) remain
   * blocked regardless. */
  allow_loopback: boolean;
};

export const DEFAULT_POLICY: SsrfPolicy = { allow_loopback: true };

export type SsrfCheck =
  | { ok: true; address: string; family: 4 | 6; hostname: string; port: number }
  | { ok: false; error: 'invalid_url' | 'blocked_host'; message: string };

export type ParsedTarget = {
  url: URL;
  hostname: string;
  port: number;
};

const DEFAULT_PORTS: Record<string, number> = { 'http:': 80, 'https:': 443 };

/** Parse + validate the URL scheme and host shape. No DNS yet. */
export function parseTarget(raw: string): ParsedTarget | { error: string } {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return { error: 'url is not a valid absolute URL' };
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return { error: `scheme not allowed: ${url.protocol} (only http: and https: are permitted)` };
  }
  if (url.hostname.length === 0) {
    return { error: 'url has no hostname' };
  }
  const portRaw = url.port.length > 0 ? Number(url.port) : DEFAULT_PORTS[url.protocol];
  if (typeof portRaw !== 'number' || !Number.isInteger(portRaw) || portRaw < 1 || portRaw > 65535) {
    return { error: `invalid port: ${url.port}` };
  }
  return { url, hostname: url.hostname, port: portRaw };
}

/**
 * IPv4 CIDR range as a packed 32-bit integer base + mask bit count.
 * Order matters for human inspection only; matching is O(N) over the
 * full list and N is small.
 */
type Cidr4 = { base: number; bits: number; label: string };

const BLOCKED_V4: readonly Cidr4[] = [
  { base: ipv4ToInt('0.0.0.0'), bits: 8, label: 'this-network' },
  { base: ipv4ToInt('10.0.0.0'), bits: 8, label: 'private rfc1918' },
  { base: ipv4ToInt('100.64.0.0'), bits: 10, label: 'cgnat rfc6598' },
  { base: ipv4ToInt('127.0.0.0'), bits: 8, label: 'loopback' },
  { base: ipv4ToInt('169.254.0.0'), bits: 16, label: 'link-local (incl. AWS metadata)' },
  { base: ipv4ToInt('172.16.0.0'), bits: 12, label: 'private rfc1918' },
  { base: ipv4ToInt('192.0.0.0'), bits: 24, label: 'ietf protocol assignments' },
  { base: ipv4ToInt('192.0.2.0'), bits: 24, label: 'documentation' },
  { base: ipv4ToInt('192.168.0.0'), bits: 16, label: 'private rfc1918' },
  { base: ipv4ToInt('198.18.0.0'), bits: 15, label: 'benchmarking' },
  { base: ipv4ToInt('198.51.100.0'), bits: 24, label: 'documentation' },
  { base: ipv4ToInt('203.0.113.0'), bits: 24, label: 'documentation' },
  { base: ipv4ToInt('224.0.0.0'), bits: 4, label: 'multicast' },
  { base: ipv4ToInt('240.0.0.0'), bits: 4, label: 'reserved' },
];

function ipv4ToInt(addr: string): number {
  const parts = addr.split('.').map(Number);
  if (parts.length !== 4 || parts.some((p) => !Number.isInteger(p) || p < 0 || p > 255)) {
    throw new Error(`bad ipv4 literal in blocklist: ${addr}`);
  }
  const a = parts[0] ?? 0;
  const b = parts[1] ?? 0;
  const c = parts[2] ?? 0;
  const d = parts[3] ?? 0;
  return ((a << 24) | (b << 16) | (c << 8) | d) >>> 0;
}

function isBlockedIpv4(
  addr: string,
  policy: SsrfPolicy = DEFAULT_POLICY,
): { blocked: boolean; label?: string } {
  let n: number;
  try {
    n = ipv4ToInt(addr);
  } catch {
    return { blocked: true, label: 'unparseable ipv4' };
  }
  for (const { base, bits, label } of BLOCKED_V4) {
    if (policy.allow_loopback && label === 'loopback') continue;
    const mask = bits === 0 ? 0 : (0xffffffff << (32 - bits)) >>> 0;
    if ((n & mask) === (base & mask)) return { blocked: true, label };
  }
  return { blocked: false };
}

/**
 * IPv6 block via prefix check on the canonical form returned by
 * `dns.lookup` — handles loopback `::1`, unspecified `::`, link-local
 * `fe80::/10`, ULA `fc00::/7`, multicast `ff00::/8`, and IPv4-mapped
 * `::ffff:X.X.X.X` (delegates the inner v4 to the v4 check).
 */
function isBlockedIpv6(
  addr: string,
  policy: SsrfPolicy = DEFAULT_POLICY,
): { blocked: boolean; label?: string } {
  const lower = addr.toLowerCase();
  if (lower === '::1') {
    if (policy.allow_loopback) return { blocked: false };
    return { blocked: true, label: 'loopback' };
  }
  if (lower === '::') return { blocked: true, label: 'unspecified' };
  if (
    lower.startsWith('fe8') ||
    lower.startsWith('fe9') ||
    lower.startsWith('fea') ||
    lower.startsWith('feb')
  ) {
    return { blocked: true, label: 'link-local fe80::/10' };
  }
  if (/^f[cd][0-9a-f]{0,2}:/.test(lower)) {
    return { blocked: true, label: 'unique-local fc00::/7' };
  }
  if (lower.startsWith('ff')) {
    return { blocked: true, label: 'multicast ff00::/8' };
  }
  // IPv4-mapped (::ffff:0:0/96) embeds a v4 address the OS routes to v4.
  // dns.lookup emits the dotted form (::ffff:1.2.3.4), but a literal URL
  // can carry the SAME address in hex (::ffff:a9fe:a9fe) — decode BOTH so
  // the v4 blocklist still applies. Decoding only the dotted form left a
  // hole: `http://[::ffff:a9fe:a9fe]/` reached 169.254.169.254 (metadata).
  const mappedV4 = decodeV4MappedIpv6(lower);
  if (mappedV4) {
    return isBlockedIpv4(mappedV4, policy);
  }
  return { blocked: false };
}

/**
 * Extract the embedded IPv4 of an IPv4-mapped IPv6 address (`::ffff:/96`),
 * in either textual form, or `null` when the address is not v4-mapped.
 * Restricted to the `::ffff:` prefix on purpose — that is the range OSes
 * actually route to v4; decoding bare `::g:g` would false-block legitimate
 * low global-unicast addresses.
 */
function decodeV4MappedIpv6(lower: string): string | null {
  const dotted = lower.match(/^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/);
  if (dotted?.[1]) return dotted[1];
  const hex = lower.match(/^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/);
  if (hex?.[1] && hex?.[2]) {
    const hi = Number.parseInt(hex[1], 16);
    const lo = Number.parseInt(hex[2], 16);
    return `${(hi >> 8) & 0xff}.${hi & 0xff}.${(lo >> 8) & 0xff}.${lo & 0xff}`;
  }
  return null;
}

/**
 * Resolve the URL's host, validate every resolved address against the
 * blocklist, and return the address to dial. If any returned address
 * is blocked the whole call is refused — defends against a DNS record
 * that returns both a public and a private answer.
 *
 * If the hostname is already a literal IP, skip DNS and check it
 * directly. This is the common case for `http://1.2.3.4/`.
 */
export async function checkTarget(
  target: ParsedTarget,
  policy: SsrfPolicy = DEFAULT_POLICY,
): Promise<SsrfCheck> {
  const { hostname, port, url } = target;

  const loopbackHint = !policy.allow_loopback
    ? ' (set web.allow_loopback=true in config.yaml if loopback is intentional in this environment)'
    : '';
  if (isIPv4(hostname)) {
    const v = isBlockedIpv4(hostname, policy);
    if (v.blocked) {
      const hint = v.label === 'loopback' ? loopbackHint : '';
      return {
        ok: false,
        error: 'blocked_host',
        message: `address ${hostname} is in ${v.label}${hint}`,
      };
    }
    return { ok: true, address: hostname, family: 4, hostname, port };
  }
  if (isIPv6(hostname.replace(/^\[|\]$/g, ''))) {
    const stripped = hostname.replace(/^\[|\]$/g, '');
    const v = isBlockedIpv6(stripped, policy);
    if (v.blocked) {
      const hint = v.label === 'loopback' ? loopbackHint : '';
      return {
        ok: false,
        error: 'blocked_host',
        message: `address ${stripped} is in ${v.label}${hint}`,
      };
    }
    return { ok: true, address: stripped, family: 6, hostname, port };
  }

  let resolved: { address: string; family: number }[];
  try {
    resolved = await dnsLookup(hostname, { all: true });
  } catch (err) {
    return {
      ok: false,
      error: 'blocked_host',
      message: `dns lookup failed for ${hostname}: ${String(err)}`,
    };
  }
  if (resolved.length === 0) {
    return {
      ok: false,
      error: 'blocked_host',
      message: `dns returned no addresses for ${hostname}`,
    };
  }

  for (const r of resolved) {
    const check =
      r.family === 4 ? isBlockedIpv4(r.address, policy) : isBlockedIpv6(r.address, policy);
    if (check.blocked) {
      const hint =
        check.label === 'loopback' && !policy.allow_loopback
          ? ' (set web.allow_loopback=true in config.yaml if loopback is intentional in this environment)'
          : '';
      return {
        ok: false,
        error: 'blocked_host',
        message: `${hostname} resolves to ${r.address} (${check.label}); refusing to dial${hint}`,
      };
    }
  }

  const first = resolved[0];
  if (!first) {
    return {
      ok: false,
      error: 'blocked_host',
      message: `dns returned no addresses for ${hostname}`,
    };
  }
  void url;
  return {
    ok: true,
    address: first.address,
    family: first.family === 4 ? 4 : 6,
    hostname,
    port,
  };
}

/** Exposed for tests. */
export const __testing = { isBlockedIpv4, isBlockedIpv6, ipv4ToInt };
