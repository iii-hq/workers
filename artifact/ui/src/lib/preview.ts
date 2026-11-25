/**
 * Turn stored content into an isolated preview document.
 *
 * The console previews in an `<iframe sandbox>` without `allow-same-origin`
 * (an opaque origin: no cookies, no storage, no access to the console) and
 * injects the same Content-Security-Policy the share listener sends, so the
 * preview behaves exactly like the shared link: inline code runs, nothing
 * loads or leaves.
 */

import type { ArtifactFormat } from './api'

const CSP_SCRIPTED =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; " +
  "img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; " +
  "form-action 'none'; base-uri 'none'"

const CSP_STATIC = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'"

/** Iframe `sandbox` tokens per format: only HTML may run script. */
export function sandboxFor(format: ArtifactFormat): string {
  return format === 'html' ? 'allow-scripts allow-forms allow-modals' : ''
}

function metaTag(csp: string): string {
  return `<meta http-equiv="Content-Security-Policy" content="${csp}">`
}

/** Put the CSP meta first inside `<head>` (or open a head for it), never
    before the doctype, which would drop the page into quirks mode. */
export function withCsp(html: string, csp: string): string {
  const meta = metaTag(csp)
  const head = /<head(\s[^>]*)?>/i.exec(html)
  if (head) {
    const at = head.index + head[0].length
    return html.slice(0, at) + meta + html.slice(at)
  }
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html)
  if (doctype) {
    const at = doctype[0].length
    return `${html.slice(0, at)}<head>${meta}</head>${html.slice(at)}`
  }
  return `<head>${meta}</head>${html}`
}

/** The srcdoc for an html or svg preview (markdown renders natively). */
export function previewDocument(format: ArtifactFormat, content: string): string {
  if (format === 'svg') {
    return withCsp(
      `<!doctype html><html><head></head><body style="margin:0;display:grid;place-items:center;min-height:100vh">${content}</body></html>`,
      CSP_STATIC,
    )
  }
  return withCsp(content, CSP_SCRIPTED)
}
