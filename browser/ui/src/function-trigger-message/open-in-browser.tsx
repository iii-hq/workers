import type { Host } from '@iii-dev/console-ui'
import { useState } from 'react'
import { startBrowserSession } from '../lib/browser'
import { ExternalLink } from '../lib/icons'
import { openBrowserPane } from '../overlay/overlay-store'

/**
 * "Open in browser" on a scrape result, the way the shell offers "View
 * file": one click starts an interactive session at the URL and opens the
 * browser page on it — the click is the decision, so no preview overlay.
 */
export function OpenInBrowser({ host, url }: { host: Host; url: string }) {
  const [state, setState] = useState<'idle' | 'opening' | 'failed'>('idle')
  return (
    <button
      type="button"
      className="br-ui-open-in-browser"
      disabled={state === 'opening'}
      aria-label={`Open ${url} in the browser page`}
      title={
        state === 'failed'
          ? 'Could not start a session; try again'
          : `Open ${url} in the browser page`
      }
      onClick={() => {
        setState('opening')
        startBrowserSession(host.iii, { url })
          .then((started) => {
            setState('idle')
            if (started) openBrowserPane(host, started.session_id)
          })
          .catch(() => setState('failed'))
      }}
    >
      <ExternalLink aria-hidden />
      <span>{state === 'opening' ? 'Opening…' : 'Open in browser'}</span>
    </button>
  )
}
