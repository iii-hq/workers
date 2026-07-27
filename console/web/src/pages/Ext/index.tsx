/**
 * The `#/ext/<page-id>` right pane — renders a worker-injected page from
 * the pages slot registry. The page body arrives pre-wrapped in its
 * `data-iii-ui` scope element + ErrorBoundary (lib/ui-loader.tsx).
 *
 * The page id comes from this component's OWN `useExtPageRoute()` — not a
 * prop from App. On a hashchange, App's view state and a prop-threaded page
 * id can land in different commits (two separate hashchange listeners), and
 * a first commit with `view === 'ext'` but a stale null id would misread as
 * "no page". Owning the hook means the mount initializer reads the live
 * hash synchronously, so the id is correct from the first render.
 *
 * If the page a user is looking at disappears (hot reload gone bad, worker
 * disconnect, explicit unregister), the router falls back to the default
 * view. A deep-link hit *before* the script loads renders a lightweight
 * notice instead of bouncing — assets typically arrive within a second of
 * the tab's `console:assets` sync.
 */

import { useEffect, useRef } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import { useExtPageRoute } from '@/hooks/use-hash-route'
import { useExtPages } from '@/lib/ui-slots'

interface ExtPageProps {
  onMissing: () => void
}

export function ExtPage({ onMissing }: ExtPageProps) {
  const pageId = useExtPageRoute()
  const pages = useExtPages()
  const page = pageId
    ? [...pages].reverse().find((p) => p.id === pageId)
    : undefined

  // Fall back to the default view ONLY when the page id this component
  // already rendered loses its registration (hot reload gone bad, worker
  // disconnect, unregister). Never on a not-loaded-yet id (renders the
  // notice below), never when the route is leaving `#/ext/*` (pageId goes
  // null a commit before the view flips — bouncing there would hijack the
  // outgoing navigation), and never when switching between ext pages.
  const seenIdRef = useRef<string | null>(null)
  useEffect(() => {
    if (page && pageId) {
      seenIdRef.current = pageId
      return
    }
    if (!pageId) {
      seenIdRef.current = null
      return
    }
    if (seenIdRef.current === pageId) {
      seenIdRef.current = null
      onMissing()
    }
  }, [page, pageId, onMissing])

  if (!page) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <EmptyState
          title="extension page not loaded"
          description={
            pageId
              ? `no worker has registered a page with id '${pageId}' (yet) — if its worker is starting up, this page appears as soon as its script loads.`
              : 'missing extension page id.'
          }
        />
      </div>
    )
  }

  const Body = page.render
  return (
    <div className="flex-1 min-h-0 overflow-y-auto">
      <Body />
    </div>
  )
}
