/**
 * Turn stored content into an isolated preview document.
 *
 * The console previews in an `<iframe sandbox>` without `allow-same-origin`
 * (an opaque origin: no cookies, no storage, no access to the console) and
 * gives the document the same Content-Security-Policy the share listener
 * sends, so the preview behaves exactly like the shared link: inline code
 * runs, nothing loads or leaves.
 *
 * The policy is never searched for and spliced into the content: a regex for
 * `<head>` also matches one inside a comment or a script string, and would
 * hide the policy there. Instead a trusted prefix opens the document and its
 * head with the policy as the very first element; the HTML parser then
 * ignores the content's own doctype, `<html>` and `<head>` tags (keeping
 * their children), so the policy always applies.
 */

import type { ArtifactFormat } from './api'

const CSP_SCRIPTED =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; " +
  "img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; " +
  "frame-src 'none'; form-action 'none'; base-uri 'none'"

const CSP_STATIC = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'"

/** Iframe `sandbox` tokens per format: only HTML may run script. */
export function sandboxFor(format: ArtifactFormat): string {
  return format === 'html' ? 'allow-scripts allow-forms allow-modals' : ''
}

/** The trusted opening every preview starts with. */
export function trustedPrefix(csp: string): string {
  return (
    '<!doctype html><html><head>' +
    `<meta http-equiv="Content-Security-Policy" content="${csp}">` +
    '<meta name="referrer" content="no-referrer">'
  )
}

/** The srcdoc for an html or svg preview (markdown renders natively). */
export function previewDocument(format: ArtifactFormat, content: string): string {
  if (format === 'svg') {
    return (
      trustedPrefix(CSP_STATIC) +
      '<style>html,body{margin:0;height:100%}body{display:grid;place-items:center}</style></head><body>' +
      content +
      '</body></html>'
    )
  }
  return trustedPrefix(CSP_SCRIPTED) + content
}
