/**
 * Address-bar url guessing, kept React-free so it is unit-testable in node.
 */

/** Hosts a browser's address bar reaches over plain http when no scheme
 * was typed: localhost, `*.localhost`, loopback and private addresses — the
 * dev-server case, where https would only fail the TLS handshake. */
export function isLocalHost(host: string): boolean {
  const bare = host.replace(/^\[|\]$/g, '').toLowerCase()
  if (bare === 'localhost' || bare.endsWith('.localhost') || bare === '::1' || bare === '0.0.0.0') return true
  const v4 = bare.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/)
  if (!v4) return false
  const [a, b] = [Number(v4[1]), Number(v4[2])]
  return a === 127 || a === 10 || (a === 192 && b === 168) || (a === 172 && b >= 16 && b <= 31) || (a === 169 && b === 254)
}

/** A typed address becomes a url: a real scheme passes, a local host gets
 * http://, anything else https:// (a bare `host:port` like localhost:3000 is
 * not a scheme). The worker still falls back to http when https fails the
 * TLS handshake on a local host. */
export function toUrl(draft: string): string {
  const url = draft.trim()
  if (!url) return ''
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(url) || url.startsWith('about:')) return url
  const host = url.split(/[/?#]/)[0]?.replace(/:\d+$/, '') ?? ''
  return `${isLocalHost(host) ? 'http' : 'https'}://${url}`
}
