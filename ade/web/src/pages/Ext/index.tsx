/**
 * A worker-injected page from the pages slot registry — a workspace-tab
 * column (`ext:<page-id>` screens) or the whole isolated `#/worker/…`
 * shell. The page body arrives pre-wrapped in its `data-iii-ui` scope
 * element + ErrorBoundary (lib/ui-loader.tsx).
 *
 * If the page a user is looking at disappears during a worker reload or
 * disconnect, its pane stays put and renders a lightweight waiting notice.
 * Assets typically return within a second of the tab's `console:assets` sync;
 * keeping the route stable prevents that transient gap from opening Traces.
 */

import { EmptyState } from '@/components/ui/EmptyState'
import { PageHeader, PageShell } from '@/components/ui/PageChrome'
import { usePanelContext } from '@/lib/panel-context'
import { useExtPages } from '@/lib/ui-slots'
import type { PageCommandsApi, PanelSide } from '@/types/injectable-ui'

interface ExtPageProps {
  /** The registered page to render. */
  pageId: string
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
  pageId,
  panelSide = 'left',
  tabId = '',
  paneId = '',
  onRequestClose,
  workingDir,
  conversationId,
  setDirty,
  commands,
}: ExtPageProps) {
  const panelContext = usePanelContext(pageId)
  const pages = useExtPages()
  const page = [...pages].reverse().find((p) => p.id === pageId)

  if (!page) {
    return (
      <PageShell aria-label={pageId}>
        <PageHeader
          title={pageId}
          description="Waiting for worker"
          onClose={onRequestClose}
        />
        <div className="flex flex-1 items-center justify-center">
          <EmptyState
            title="Extension page not loaded"
            description={`No worker has registered a page with id '${pageId}' (yet) — if its worker is starting up, this page appears as soon as its script loads.`}
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
