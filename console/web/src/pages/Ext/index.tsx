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
 * If the page a user is looking at disappears during a worker reload or
 * disconnect, its pane stays put and renders a lightweight waiting notice.
 * Assets typically return within a second of the tab's `console:assets` sync;
 * keeping the route stable prevents that transient gap from opening Traces.
 */

import { EmptyState } from '@/components/ui/EmptyState'
import { PageHeader, PageShell } from '@/components/ui/PageChrome'
import { useExtPageRoute } from '@/hooks/use-hash-route'
import { usePanelContext } from '@/lib/panel-context'
import { useExtPages } from '@/lib/ui-slots'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'

interface ExtPageProps {
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
   * Stable id of the hosting pane inside that tab — forwarded so a page
   * opened twice in one tab keys per-instance state. Empty outside a tab.
   */
  paneId?: string
  /**
   * Close the hosting pane — forwarded so the page's `PageHeader` ✕ works.
   * Absent when rendered outside a closable pane.
   */
  onRequestClose?: () => void
  /**
   * The active chat conversation's working directory — forwarded live so
   * filesystem-shaped pages can follow the chat's folder.
   */
  workingDir?: string | null
  /** Active chat id for session-scoped reactive pages. */
  conversationId?: string | null
  /** Report unsaved work so closing the pane or workspace asks first. */
  setDirty?: (dirty: boolean | string) => void
  commands?: PageCommandsApi
}

export function ExtPage({
  pageId: pageIdProp,
  panelSide = 'left',
  tabId = '',
  paneId = '',
  onRequestClose,
  workingDir,
  conversationId,
  setDirty,
  commands,
}: ExtPageProps) {
  const routePageId = useExtPageRoute()
  const pageId = pageIdProp ?? routePageId
  const panelContext = usePanelContext(pageId ?? '')
  const pages = useExtPages()
  const page = pageId
    ? [...pages].reverse().find((p) => p.id === pageId)
    : undefined

  if (!page) {
    return (
      <PageShell aria-label={pageId ?? 'extension page'}>
        <PageHeader
          title={pageId ?? 'Extension'}
          description="Waiting for worker"
          onClose={onRequestClose}
        />
        <div className="flex flex-1 items-center justify-center">
          <EmptyState
            title="Extension page not loaded"
            description={
              pageId
                ? `No worker has registered a page with id '${pageId}' (yet) — if its worker is starting up, this page appears as soon as its script loads.`
                : 'Missing extension page id.'
            }
          />
        </div>
      </PageShell>
    )
  }

  const Body = page.render
  return (
    <div className="flex-1 min-h-0 overflow-y-auto">
      <Body
        panelSide={panelSide}
        tabId={tabId}
        paneId={paneId}
        onRequestClose={onRequestClose}
        workingDir={workingDir}
        panelContext={panelContext}
        conversationId={conversationId}
        setDirty={setDirty}
        commands={commands}
      />
    </div>
  )
}
