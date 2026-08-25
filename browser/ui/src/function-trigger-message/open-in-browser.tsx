import type { Host } from '@iii-dev/console-ui'
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
      title={
        state === 'failed'
          ? 'could not start a session; try again'
          : `open ${url} in the browser page`
      }
      onClick={() => {
        setState('opening')
        host.iii
          .trigger('browser::sessions::start', { url })
          .then(() => setState('idle'))
          .catch(() => setState('failed'))
      }}
    >
      {state === 'opening' ? 'opening…' : 'open in browser'}
    </button>
  )
}
