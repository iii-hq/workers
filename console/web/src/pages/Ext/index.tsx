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
import type { PanelSide } from '@/types/injectable-ui'

interface ExtPageProps {
  onMissing: () => void
  /**
   * Render this specific page instead of the hash-derived one. Workspace
   * tabs pin a screen to a page id, so a two-column tab can show an
   * injected page regardless of what the hash currently names.
   */
  pageId?: string
  /**
   * Which side of the workspace tab this pane occupies — forwarded to the
   * page render so extensions can mirror their layout (e.g. put a sidebar
   * against the outer edge). Defaults to `'left'`, the single-column case.
   */
  panelSide?: PanelSide
  /**
   * Stable id of the hosting workspace tab — forwarded so extensions can
   * key per-tab UI state. Empty when rendered outside a workspace tab.
   */
  tabId?: string
  /**
   * Close the hosting pane — forwarded so the page's `PageHeader` ✕ works.
   * Absent when rendered outside a closable pane.
   */
  onRequestClose?: () => void
}

export function ExtPage({
  onMissing,
  pageId: pageIdProp,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
}: ExtPageProps) {
  const routePageId = useExtPageRoute()
  const pageId = pageIdProp ?? routePageId
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
      <Body
        panelSide={panelSide}
        tabId={tabId}
        onRequestClose={onRequestClose}
      />
    </div>
  )
}
