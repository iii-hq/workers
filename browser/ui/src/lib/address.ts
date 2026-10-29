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

/** Where the address bar sends a query that is not an address. */
export const SEARCH_URL = 'https://www.google.com/search?q='

/** A typed address becomes a url: a real scheme passes, a local host gets
 * http://, a host-looking word (a dot, a port, `localhost`) gets https://,
 * and anything else — words with spaces, a lone word — is a search, the way
 * a browser's address bar reads it (a bare `host:port` like localhost:3000
 * is not a scheme). The worker still falls back to http when https fails
 * the TLS handshake on a local host. */
export function toUrl(draft: string): string {
  const url = draft.trim()
  if (!url) return ''
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(url) || url.startsWith('about:')) return url
  const hostPort = url.split(/[/?#]/)[0] ?? ''
  const host = hostPort.replace(/:\d+$/, '')
  const looksLikeHost =
    !/\s/.test(url) &&
    host.length > 0 &&
    (isLocalHost(host) || /^\[[0-9a-f:]+\]$/i.test(host) || /:\d+$/.test(hostPort) || /^[^.\s]+(\.[^.\s]+)+$/.test(host))
  if (!looksLikeHost) return `${SEARCH_URL}${encodeURIComponent(url)}`
  return `${isLocalHost(host) ? 'http' : 'https'}://${url}`
}

/** How the bar reads a page it is not editing, the way Chrome shows it: the
 * scheme dropped for http(s), the host in full ink and the rest (path,
 * query, hash) dimmed; a bare root `/` is dropped. Other schemes (file:,
 * data:, about:) read whole, all dimmed. */
export function displayUrl(url: string): { host: string; rest: string } {
  // Plain string parsing (no `URL`): the iii app's native viewer shares this
  // file and has no DOM library.
  const match = /^https?:\/\/([^/?#]+)(.*)$/i.exec(url)
  if (!match) return { host: '', rest: url }
  const tail = match[2] ?? ''
  return { host: match[1] ?? '', rest: tail === '/' ? '' : tail }
}

/** What the bar's site icon says about a page: https is secure, http is
 * not (except on a local host, where it is just a dev server), a file is a
 * file, and an empty tab has nothing to say. */
export type SiteSecurity = 'secure' | 'insecure' | 'local' | 'file' | 'none'

export function siteSecurity(url: string): SiteSecurity {
  if (!url || url === 'about:blank') return 'none'
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(url)?.[1]?.toLowerCase()
  if (scheme === 'https') return 'secure'
  if (scheme === 'http') {
    const authority = /^http:\/\/([^/?#]+)/i.exec(url)?.[1] ?? ''
    const host = authority.replace(/^[^@]*@/, '').replace(/:\d+$/, '')
    return isLocalHost(host) ? 'local' : 'insecure'
  }
  if (scheme === 'file') return 'file'
  return 'none'
}
