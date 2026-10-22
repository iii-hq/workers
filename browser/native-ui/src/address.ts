/**
 * The address bar's rules. What a typed address becomes is the console UI's
 * own rule (D10: URLs only, no search), imported so both bars agree; the
 * rest is how a URL reads on a phone: without its scheme, the host as the
 * subtitle, the lock on https. No SDK import: unit-testable in node.
 */

export { isLocalHost, toUrl } from '../../ui/src/lib/address'

const URL_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/

const BLANK = 'about:blank'

/** The host of an absolute URL (port kept, credentials dropped, lower case); '' when there is none. */
export function hostOf(url: string): string {
  const m = URL_RE.exec(String(url ?? '').trim())
  if (!m || m[2] === undefined) return ''
  return m[2].replace(/^[^@]*@/, '').toLowerCase()
}

/** The URL as the pill and the rows show it: http(s) without its scheme; '' for a blank tab. */
export function displayUrl(url: string): string {
  const u = String(url ?? '').trim()
  if (!u || u === BLANK) return ''
  return u.replace(/^https?:\/\//i, '')
}

export function isSecure(url: string): boolean {
  return /^https:\/\//i.test(String(url ?? '').trim())
}

/** Whether the system browser can open it (http and https). */
export function isWebUrl(url: string): boolean {
  return /^https?:\/\//i.test(String(url ?? '').trim())
}

export function isBlank(url: string): boolean {
  const u = String(url ?? '').trim()
  return !u || u === BLANK
}

/** A tab's name: its title, else its host, else "New tab". */
export function tabTitle(title: string | undefined, url: string): string {
  const t = (title ?? '').trim()
  if (t && t !== BLANK && t !== url) return t
  return hostOf(url) || (isBlank(url) ? 'New tab' : displayUrl(url) || 'New tab')
}
