/**
 * Provider marks travel as inline SVG in `router::provider::list`
 * (`icon_svg`, declared by each provider worker at registration). The console
 * never injects that markup into the document: it paints the SVG as a CSS
 * mask filled with `currentColor`, so a mark can carry no script or style,
 * follows the theme like a Lucide glyph, and stays monochrome the way the
 * picker's provider rail wants it.
 */

/** Anything larger is not an icon; the router drops oversized declarations. */
export const PROVIDER_ICON_SVG_MAX_BYTES = 32 * 1024

const LEADING_PROLOG =
  /^(?:\s|<\?xml[^>]*\?>|<!--[\s\S]*?-->|<!DOCTYPE[^>]*>)+/i
const SVG_NAMESPACE = 'http://www.w3.org/2000/svg'

/**
 * Strip the XML prolog, comments, and doctype so the payload starts at the
 * root `<svg>`; add the SVG namespace when the mark omits it (an image loaded
 * from a data URL renders nothing without one). `null` when the string is
 * not a self-contained SVG document of a sane size.
 */
export function normalizeProviderIconSvg(
  svg: string | null | undefined,
): string | null {
  if (typeof svg !== 'string') return null
  const body = svg.replace(LEADING_PROLOG, '').trim()
  if (body.length === 0 || body.length > PROVIDER_ICON_SVG_MAX_BYTES) {
    return null
  }
  const openTag = body.match(/^<svg(?:\s[^>]*)?>/i)?.[0]
  if (!openTag || !/<\/svg>$/i.test(body)) return null
  if (/\sxmlns\s*=/.test(openTag)) return body
  return `<svg xmlns="${SVG_NAMESPACE}"${body.slice('<svg'.length)}`
}

/** `url("data:…")` for `mask-image`; `null` when the mark is unusable. */
export function providerIconMaskUrl(
  svg: string | null | undefined,
): string | null {
  const body = normalizeProviderIconSvg(svg)
  if (!body) return null
  return `url("data:image/svg+xml;charset=utf-8,${encodeURIComponent(body)}")`
}

/** Fallback glyph for a provider without a mark: its first letter or digit. */
export function providerInitial(label: string): string {
  const trimmed = label.trim()
  const first = trimmed.match(/[\p{L}\p{N}]/u)?.[0] ?? trimmed[0] ?? '?'
  return first.toUpperCase()
}
