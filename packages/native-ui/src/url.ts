/** URL parsing without the `URL` global (the VM has none, design §4.1). */

const URL_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/

/**
 * The parts of an absolute URL, like `URL`'s (`host` keeps the port, drops
 * credentials, lower case); null when `url` has no scheme.
 */
export function parseUrl(
  url: string,
): { protocol: string; host: string; pathname: string; search: string; hash: string } | null {
  const m = URL_RE.exec(String(url).trim())
  if (!m) return null
  const authority = m[2]
  return {
    protocol: m[1].toLowerCase(),
    host: authority === undefined ? '' : authority.replace(/^[^@]*@/, '').toLowerCase(),
    pathname: m[3] || (authority === undefined ? '' : '/'),
    search: m[4] && m[4] !== '?' ? m[4] : '',
    hash: m[5] && m[5] !== '#' ? m[5] : '',
  }
}
