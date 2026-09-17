import { useCallback, useEffect, useMemo, useRef } from 'react'
import { ExtOverlays } from '@/components/ExtOverlays'
import { Sheet } from '@/components/ui/Sheet'
import {
  hashForSettingsLanding,
  hashForStandalone,
  hashForWorkerPage,
  replaceHash,
  routeFromHash,
  type StandaloneRoute,
  standaloneRouteFromHash,
  useHash,
  type WorkerRoute,
} from '@/hooks/use-hash-route'
import { useKeybindings } from '@/hooks/use-keybindings'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useTheme } from '@/hooks/use-theme'
import {
  ConversationsProvider,
  type InjectableUiRuntime,
  useConversationsCtx,
} from '@/lib/conversations-context'
import { requestPanelOpen } from '@/lib/panel-context'
import { useExtPages } from '@/lib/ui-slots'
import { useUnsavedGuard } from '@/pages/Configuration/tabs/WorkersTab/useUnsavedGuard'
import { ExtPage } from '@/pages/Ext'
import { TracesV2 } from '@/pages/TracesV2'
import { ConfigurationOverlay, usePaneCommandsApi } from './App'

/** The one pane root: a page's keyed commands dispatch to the focused pane. */
const PANE_ID = 'standalone'

/**
 * The standalone shell: `#/worker/<scope>[/<page-id>][?context=<json>]`
 * renders ONE worker-injected page and `#/traces` the traces explorer, over
 * the full viewport — no tab strip, chat or palette — for visual tests of
 * that surface alone. The workspace store is never read or written here, so
 * opening these URLs leaves the shared layout untouched. What does carry
 * over from the app: the loader (hot reload, the worker's overlays), the
 * page's settings action and the settings shortcut (the configuration
 * overlay opens in place; `#/configuration…` keeps this surface underneath),
 * and the page's keyed commands. `host.panels.open` for another page opens
 * a new browser tab (lib/ui-loader.tsx).
 */
export function Standalone({
  injectableUiRuntime,
}: {
  injectableUiRuntime?: Promise<InjectableUiRuntime>
}) {
  const [theme, setTheme] = useTheme()
  const hash = useHash()
  const live = useMemo(() => standaloneRouteFromHash(hash), [hash])
  // Settings swap the hash for `#/configuration…`: the surface underneath
  // stays the last standalone route until the overlay closes.
  const lastRef = useRef(live)
  if (live) lastRef.current = live
  const route = lastRef.current
  const settingsOpen = routeFromHash(hash) === 'configuration'
  const { setDirty, tryNavigate } = useUnsavedGuard({
    guardHashNavigation: true,
  })
  const closeSettings = useCallback(() => {
    if (route) window.location.replace(hashForStandalone(route))
  }, [route])
  const narrowSettings = useMediaQuery('(max-width: 639px)')
  useKeybindings({
    'app.settings': () => {
      if (settingsOpen) tryNavigate(closeSettings)
      else window.location.hash = hashForSettingsLanding(narrowSettings)
    },
  })
  const name = route?.kind === 'traces' ? 'traces' : (route?.scope ?? 'ade')
  useEffect(() => {
    document.title = `iii - ${name}`
  }, [name])
  return (
    <ConversationsProvider injectableUiRuntime={injectableUiRuntime}>
      <Sheet>
        <div
          className="flex min-h-0 flex-1 flex-col"
          data-workspace-pane-id={PANE_ID}
        >
          {route?.kind === 'traces' ? (
            <StandaloneTraces />
          ) : route ? (
            <WorkerPage route={route} />
          ) : null}
        </div>
        <ExtOverlays />
        {settingsOpen ? (
          <ConfigurationOverlay
            theme={theme}
            onThemeChange={setTheme}
            onDirtyChange={setDirty}
            tryNavigate={tryNavigate}
            onClose={() => tryNavigate(closeSettings)}
          />
        ) : null}
      </Sheet>
    </ConversationsProvider>
  )
}

function StandaloneTraces() {
  const commands = usePaneCommandsApi('traces', 'Traces', PANE_ID)
  return <TracesV2 commands={commands} />
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
  const title = pages.find((page) => page.id === pageId)?.title
  const commands = usePaneCommandsApi(pageId, title, PANE_ID)
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
      commands={commands}
    />
  )
}

export type { StandaloneRoute }
