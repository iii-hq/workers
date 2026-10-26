/**
 * The address bar's rules. What a typed address becomes is the console UI's
 * own rule (`toUrl`, imported so both bars agree); on top of it the native
 * bar opens web addresses only (D10): http or https with a host, never a
 * control character. The rest is how a URL reads on a phone: without its
 * scheme, the host as the subtitle, the lock on https. No SDK import:
 * unit-testable in node.
 */

import { toUrl } from '../../ui/src/lib/address'

export { isLocalHost, toUrl } from '../../ui/src/lib/address'

const URL_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#[\s\S]*)?$/

/** A host as a web address has it: a name (no space, colon, slash, ?, #, @ or bracket) or a bracketed IPv6, then an optional numeric port. */
const WEB_HOST = /^(?:\[[0-9a-f:.]+\]|[^\s:/?#[\]@]+)(?::\d{1,5})?$/i

/** C0 and C1 controls, DEL, and the bidi marks, embeddings, overrides and isolates. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching control characters is what it is for
const CONTROLS = /[\u0000-\u001f\u007f-\u009f\u200e\u200f\u202a-\u202e\u2066-\u2069]/g

const BLANK = 'about:blank'

/** What the person or the worker wrote, without the characters that only hide or reorder text. */
export function stripControls(text: unknown): string {
  return typeof text === 'string' ? text.replace(CONTROLS, '') : ''
}

/** The host of an absolute URL (port kept, credentials dropped, lower case); '' when there is none. */
export function hostOf(url: string): string {
  const m = URL_RE.exec(stripControls(url).trim())
  if (!m || m[2] === undefined) return ''
  return m[2].replace(/^[^@]*@/, '').toLowerCase()
}

/**
 * An absolute URL the app may open (D10: web addresses only): http or https
 * with a well-formed host, without control characters; null otherwise. `javascript:`,
 * `data:`, `file:`, `chrome:`, `intent:`, `about:` and the like never reach
 * the worker from the app. Every navigation (the bar, Retry, a history pick)
 * passes here.
 */
export function webUrl(url: unknown): string | null {
  const u = stripControls(url).trim()
  if (!/^https?:\/\//i.test(u)) return null
  return WEB_HOST.test(hostOf(u)) ? u : null
}

/** What a typed address becomes on the bar: the console UI's `toUrl` adds the scheme, then `webUrl` decides. */
export function webAddress(draft: unknown): string | null {
  return webUrl(toUrl(stripControls(draft)))
}

/** What the app says when it refuses an address. */
export const NOT_WEB = 'Only web addresses open here'

/** The URL as the pill and the rows show it: http(s) without its scheme; '' for a blank tab. */
export function displayUrl(url: string): string {
  const u = stripControls(url).trim()
  if (!u || u === BLANK) return ''
  return u.replace(/^https?:\/\//i, '')
}

export function isSecure(url: string): boolean {
  return /^https:\/\//i.test(stripControls(url).trim())
}

/** Whether the system browser can open it (http and https). */
export function isWebUrl(url: string): boolean {
  return /^https?:\/\//i.test(stripControls(url).trim())
}

export function isBlank(url: string): boolean {
  const u = stripControls(url).trim()
  return !u || u === BLANK
}

/** A tab's name: its title (without controls), else its host, else "New tab". */
export function tabTitle(title: unknown, url: unknown): string {
  const u = stripControls(url)
  const t = stripControls(title).trim()
  if (t && t !== BLANK && t !== u) return t
  return hostOf(u) || (isBlank(u) ? 'New tab' : displayUrl(u) || 'New tab')
}
