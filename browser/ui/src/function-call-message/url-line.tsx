import { ActionLine, type Host } from '@iii-dev/console-ui'
import { ArrowRight } from 'lucide-react'
import { cn } from '../lib/cn'
import { OpenInBrowser } from './open-in-browser'

/**
 * The `→ url` line a browser card reports. An http(s) URL reads as a link
 * (opens in a new tab); with a host, "Open in browser" sits at the trailing
 * edge. The arrow, the URL's first line and the button share one centred
 * band (`.br-ui-url-line` in styles.css), so nothing rides high or low.
 */
export function UrlLine({ url, host }: { url: string; host?: Host }) {
  const href = linkHref(url)
  const actionable = !!host && url.length > 0
  return (
    <ActionLine
      className={cn('br-ui-url-line', actionable && 'has-action')}
      icon={<ArrowRight size={16} aria-hidden />}
      tone="ink"
    >
      {href ? (
        <a
          className="br-ui-url br-ui-link br-ui-break"
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          title={`Open ${url} in a new tab`}
        >
          {url}
        </a>
      ) : (
        <span className="br-ui-url br-ui-break">{url}</span>
      )}
      {actionable && host ? <OpenInBrowser host={host} url={url} /> : null}
    </ActionLine>
  )
}

/** Only http(s) URLs become anchors; anything else stays plain text. */
export function linkHref(url: string): string | null {
  if (!/^https?:\/\//i.test(url)) return null
  try {
    return new URL(url).href
  } catch {
    return null
  }
}
