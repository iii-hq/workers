import type { Host } from '@iii-dev/console-ui'
import { AppWindow } from 'lucide-react'
import { useState } from 'react'

/**
 * "Open in browser" on a scrape result, the way the shell offers "View
 * file": one click starts an interactive session at the URL, and the
 * session-started binding pulls the browser page into the workspace with
 * that session selected.
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
        host.iii
          .trigger('browser::sessions::start', { url })
          .then(() => setState('idle'))
          .catch(() => setState('failed'))
      }}
    >
      <AppWindow aria-hidden />
      <span>{state === 'opening' ? 'Opening…' : 'Open in browser'}</span>
    </button>
  )
}
