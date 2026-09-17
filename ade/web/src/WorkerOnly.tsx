import { useEffect } from 'react'
import { ExtOverlays } from '@/components/ExtOverlays'
import { Sheet } from '@/components/ui/Sheet'
import {
  hashForWorkerPage,
  replaceHash,
  useWorkerRoute,
  type WorkerRoute,
} from '@/hooks/use-hash-route'
import { useTheme } from '@/hooks/use-theme'
import {
  ConversationsProvider,
  type InjectableUiRuntime,
  useConversationsCtx,
} from '@/lib/conversations-context'
import { requestPanelOpen } from '@/lib/panel-context'
import { useExtPages } from '@/lib/ui-slots'
import { ExtPage } from '@/pages/Ext'

/**
 * The isolated shell: `#/worker/<scope>[/<page-id>][?context=<json>]`
 * renders ONE worker-injected page over the full viewport — no tab strip,
 * chat, palette or keybindings — for visual tests of that worker alone.
 * The workspace store is never read or written here, so opening this URL
 * leaves the shared layout untouched. The loader still runs (through
 * `ConversationsProvider`), so hot reload and the worker's overlays work as
 * in the app; `host.panels.open` for another page opens a new browser tab
 * (lib/ui-loader.tsx).
 */
export function WorkerOnly({
  injectableUiRuntime,
}: {
  injectableUiRuntime?: Promise<InjectableUiRuntime>
}) {
  useTheme()
  const route = useWorkerRoute()
  return (
    <ConversationsProvider injectableUiRuntime={injectableUiRuntime}>
      <Sheet>
        {route ? <WorkerPage route={route} /> : null}
        <ExtOverlays />
      </Sheet>
    </ConversationsProvider>
  )
}

function WorkerPage({ route }: { route: WorkerRoute }) {
  const pages = useExtPages()
  const { active } = useConversationsCtx()
  // A bare `#/worker/<scope>` means the worker's first page; until its
  // script loads, the scope stands in and ExtPage shows its waiting notice.
  const pageId =
    route.pageId ??
    pages.find((page) => page.scope === route.scope)?.id ??
    route.scope
  // Canonical URL once resolved, so `host.panels.open` can tell "this page"
  // from "another page" by reading the hash alone.
  useEffect(() => {
    if (route.pageId === null && pageId !== route.scope) {
      replaceHash(hashForWorkerPage(route.scope, pageId, route.context))
    }
  }, [route, pageId])
  // The context `host.panels.open` would have delivered, replayed from the
  // URL on every route change (and on reload).
  useEffect(() => {
    if (route.pageId !== null && route.context !== null) {
      requestPanelOpen({ pageId: route.pageId, context: route.context })
    }
  }, [route])
  return (
    <ExtPage
      pageId={pageId}
      workingDir={active?.workingDir ?? null}
      conversationId={active?.id ?? null}
    />
  )
}
