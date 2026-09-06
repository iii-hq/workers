/**
 * The browser page (#/ext/browser): a browser. Under the standard pane
 * header sits a Chrome-style tab strip over the selected tab's workspace —
 * address bar, a screencast-fed live viewport that fills the pane, and the
 * developer tools (console, network, downloads, history) docked below only
 * when asked for from the ⋮ menu. A user watches what an agent is doing in a
 * tab, drives the page directly, and pins elements for the chat.
 *
 * Tabs are the worker's: `browser::sessions::list` is the strip, re-read on
 * every lifecycle trigger (opened, closed, slept, woke, navigated). A tab
 * the worker put to sleep is still listed; selecting it starts its
 * screencast, which wakes it. The selected tab is page-local state kept per
 * workspace tab, so a reload lands where it left off.
 *
 * The host only mounts this page while the browser worker is connected, so
 * there is no presence gate here (worker disconnect disposes the script and
 * drops the nav entry).
 */

import {
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  errorMessage,
  startBrowserSession,
  stopBrowserSession,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { Globe, Incognito, Plus } from '../lib/icons'
import { GlobeIcon, useContainerNarrow } from '../lib/widgets'
import { SavedSetsDialog } from './SavedSetsDialog'
import { type SessionActions, SessionView } from './SessionView'
import { TabStrip } from './TabStrip'
import { useBrowserSessionsLive } from './useBrowserSessionsLive'

/** Container width (px) below which controls grow to touch size. */
const NARROW_BELOW = 720

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeStored(key: string, value: string | null) {
  try {
    if (value === null) window.localStorage.removeItem(key)
    else window.localStorage.setItem(key, value)
  } catch {
    /* private mode / quota — persistence is best-effort */
  }
}

export function BrowserPage({
  host,
  tabId = '',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const {
    sessions: tabs,
    loading,
    error,
    refresh,
  } = useBrowserSessionsLive(host, true)
  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)

  // The selected tab survives a reload of the console.
  const selectionKey = `browser-ui:${tabId || 'page'}:tab`
  const [selectedId, setSelectedIdState] = useState<string | null>(() =>
    readStored(selectionKey),
  )
  const setSelectedId = useCallback(
    (next: string | null | ((current: string | null) => string | null)) => {
      setSelectedIdState((current) => {
        const value = typeof next === 'function' ? next(current) : next
        writeStored(selectionKey, value)
        return value
      })
    },
    [selectionKey],
  )

  // A tab selected the moment it opens is not in the list yet; hold it until
  // the refresh lands so the selection does not bounce to another tab and
  // then back again.
  const pendingIdRef = useRef<string | null>(null)

  // Fall back to the first tab when nothing (or a closed tab) is selected.
  useEffect(() => {
    if (loading) return
    setSelectedId((current) => {
      if (current && tabs.some((t) => t.session_id === current)) {
        pendingIdRef.current = null
        return current
      }
      if (current && current === pendingIdRef.current) return current
      pendingIdRef.current = null
      return tabs[0]?.session_id ?? null
    })
  }, [loading, tabs, setSelectedId])

  const selected = useMemo(
    () => tabs.find((t) => t.session_id === selectedId) ?? null,
    [tabs, selectedId],
  )

  const [starting, setStarting] = useState(false)
  const [startError, setStartError] = useState<string | null>(null)
  const startingRef = useRef(false)
  const handleNewTab = useCallback(
    async (incognito = false) => {
      if (startingRef.current) return
      startingRef.current = true
      setStarting(true)
      try {
        const started = await startBrowserSession(host.iii, { incognito })
        setStartError(null)
        refresh()
        if (started) {
          pendingIdRef.current = started.session_id
          setSelectedId(started.session_id)
        }
      } catch (err) {
        setStartError(errorMessage(err))
      } finally {
        startingRef.current = false
        setStarting(false)
      }
    },
    [host, refresh, setSelectedId],
  )

  // A browser opens with a tab: the first visit to an empty strip makes one.
  // Once only, so closing the last tab leaves the empty state, not a loop.
  const autoOpenedRef = useRef(false)
  useEffect(() => {
    if (loading || error || tabs.length > 0 || autoOpenedRef.current) return
    autoOpenedRef.current = true
    void handleNewTab(false)
  }, [loading, error, tabs.length, handleNewTab])

  // Closing the selected tab lands on its right-hand neighbour, else the
  // left one — the way every browser does it.
  const closeTab = useCallback(
    (sessionId: string) => {
      const index = tabs.findIndex((t) => t.session_id === sessionId)
      const neighbour = tabs[index + 1] ?? tabs[index - 1] ?? null
      if (sessionId === selectedId) {
        setSelectedId(neighbour?.session_id ?? null)
      }
      void stopBrowserSession(host.iii, sessionId)
        .then(() => refresh())
        .catch((err: unknown) => setStartError(errorMessage(err)))
    },
    [host, tabs, selectedId, refresh, setSelectedId],
  )

  // Per-tab verbs (annotate, find, zoom, devtools…) live inside SessionView;
  // this ref lets page-level commands reach the mounted tab's handlers
  // without lifting their state up.
  const sessionActionsRef = useRef<SessionActions | null>(null)
  const [savedSets, setSavedSets] = useState<{
    open: boolean
    key: string | null
  }>({ open: false, key: null })
  const openSavedSets = useCallback(
    (key: string | null = null) => setSavedSets({ open: true, key }),
    [],
  )

  // A palette row (or any other host.panels.open caller) selects a tab, or
  // asks for a new one, through the standard panelContext channel.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context =
      panelContext.context &&
      typeof panelContext.context === 'object' &&
      !Array.isArray(panelContext.context)
        ? (panelContext.context as Record<string, unknown>)
        : {}
    if (typeof context.sessionId === 'string' && context.sessionId) {
      setSelectedId(context.sessionId)
    }
    if (context.type === 'saved-set' && typeof context.key === 'string') {
      openSavedSets(context.key)
    }
    if (context.type === 'new-tab') {
      void handleNewTab(context.incognito === true)
    }
  }, [panelContext, openSavedSets, handleNewTab, setSelectedId])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-tab',
          title: 'New tab',
          keywords: ['open', 'start', 'session'],
          run: () => void handleNewTab(false),
        },
        {
          id: 'new-incognito-tab',
          title: 'New incognito tab',
          detail: 'A private tab: nothing saved, closes when idle',
          keywords: ['private', 'incognito', 'session'],
          run: () => void handleNewTab(true),
        },
        {
          id: 'close-tab',
          title: 'Close tab',
          detail: 'Close the selected tab',
          keywords: ['stop', 'end', 'session'],
          enabled: () => selected !== null,
          run: () => {
            if (selectedId) closeTab(selectedId)
          },
        },
        {
          id: 'developer-tools',
          title: 'Toggle developer tools',
          detail: 'Console, network, downloads and history for this tab',
          keywords: ['console', 'network', 'devtools', 'inspect', 'logs'],
          shortcut: 'Mod+Shift+I',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.toggleDevtools(),
        },
        {
          id: 'annotate',
          title: 'Annotate the view',
          detail: 'Freeze the live view and pin elements with notes',
          keywords: [
            'pin',
            'inspect',
            'pick',
            'element',
            'comment',
            'markup',
            'feedback',
            'screenshot',
          ],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.toggleAnnotate(),
        },
        {
          id: 'send-annotations',
          title: 'Send annotations to the chat',
          detail: 'Attach each pin to the conversation, plus the whole view',
          keywords: ['annotations', 'chat', 'share', 'send'],
          shortcut: 'Mod+Enter',
          firesWhileTyping: true,
          enabled: () =>
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.sendAnnotations(),
        },
        {
          id: 'download-annotations',
          title: 'Download the annotated picture',
          detail: 'Save the frozen view with its pins as a PNG',
          keywords: ['annotations', 'save', 'png', 'export'],
          enabled: () =>
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.downloadAnnotations(),
        },
        {
          id: 'find-in-page',
          title: 'Find in page',
          detail: 'Search the page text; Enter steps, Escape closes',
          keywords: ['search', 'text', 'match'],
          shortcut: 'Mod+F',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.findInPage(),
        },
        {
          id: 'zoom-in',
          title: 'Zoom in',
          keywords: ['zoom', 'bigger', 'scale'],
          shortcut: 'Mod+=',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.zoom('in'),
        },
        {
          id: 'zoom-out',
          title: 'Zoom out',
          keywords: ['zoom', 'smaller', 'scale'],
          shortcut: 'Mod+-',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.zoom('out'),
        },
        {
          id: 'zoom-reset',
          title: 'Reset zoom',
          detail: 'Back to 100 %',
          keywords: ['zoom', 'actual', 'size'],
          shortcut: 'Mod+0',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.zoom('reset'),
        },
        {
          id: 'screenshot',
          title: 'Take a screenshot',
          detail: 'Save the page as a JPEG',
          keywords: ['capture', 'image', 'download'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.takeScreenshot(),
        },
        {
          id: 'screenshot-to-chat',
          title: 'Screenshot to chat',
          detail: 'Attach the page as an image to the conversation',
          keywords: ['capture', 'image', 'attach', 'send'],
          enabled: () =>
            selected !== null &&
            sessionActionsRef.current !== null &&
            typeof host.chat?.compose === 'function',
          run: () => sessionActionsRef.current?.screenshotToChat(),
        },
        {
          id: 'print-pdf',
          title: 'Print to PDF',
          detail: 'Save the page as a PDF',
          keywords: ['print', 'pdf', 'save', 'export'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.printToPdf(),
        },
        {
          id: 'clear-site-data',
          title: 'Clear cookies and site data',
          detail: "This tab's site only; other sites stay signed in",
          keywords: ['clear', 'cookies', 'cache', 'storage', 'reset', 'logout'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.clearSiteData(),
        },
        {
          id: 'device-toolbar',
          title: 'Toggle device toolbar',
          detail: 'Pin the viewport to a device size',
          keywords: ['device', 'responsive', 'mobile', 'viewport', 'width'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.toggleDeviceToolbar(),
        },
        {
          id: 'import-cookies',
          title: 'Import cookies',
          detail: 'Load a JSON or Netscape cookie file into the browser',
          keywords: ['cookies', 'import', 'auth', 'session'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.importCookies(),
        },
        {
          id: 'copy-cookies',
          title: 'Copy cookies',
          detail: "Copy this site's cookies as JSON",
          keywords: ['cookies', 'export', 'copy'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.copyCookies(),
        },
        {
          id: 'save-annotations',
          title: 'Save the annotations',
          detail: 'Keep this set; reopen or send it later from Saved annotations',
          keywords: ['annotations', 'save', 'keep'],
          enabled: () =>
            selected !== null &&
            sessionActionsRef.current !== null &&
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.saveSet(),
        },
        {
          id: 'saved-annotations',
          title: 'Saved annotations',
          detail: 'Sets saved from any tab; send or download them again',
          keywords: ['annotations', 'saved', 'sets', 'share'],
          run: () => openSavedSets(),
        },
        {
          id: 'browser-diagnostics',
          title: 'Browser diagnostics',
          detail: 'What Chromium the worker launches and what it allows',
          keywords: ['doctor', 'diagnostics', 'environment', 'chromium'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.showDiagnostics(),
        },
        {
          id: 'clear-annotations',
          title: 'Clear the annotations',
          keywords: ['annotations', 'remove', 'reset'],
          enabled: () =>
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.clearAnnotations(),
        },
        {
          id: 'focus-url',
          title: 'Focus the address bar',
          detail: 'Type a url to navigate the selected tab',
          keywords: ['address', 'navigate', 'url'],
          shortcut: 'Mod+L',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.focusUrl(),
        },
      ]),
    [commands, handleNewTab, closeTab, selected, selectedId, openSavedSets, host],
  )

  const banner = error ?? startError

  return (
    <PageShell className="br-ui-shell">
      <PageHeader
        icon={<GlobeIcon />}
        title="Browser"
        onClose={onRequestClose}
      />

      {banner ? (
        <div className="br-ui-banner alert" role="alert">
          <span>{banner}</span>
          {error ? (
            <button type="button" className="br-ui-linkish" onClick={refresh}>
              retry
            </button>
          ) : (
            <button
              type="button"
              className="br-ui-linkish quiet"
              onClick={() => setStartError(null)}
            >
              dismiss
            </button>
          )}
        </div>
      ) : null}

      <div
        className={cn(
          'br-ui-browser',
          narrow && 'narrow',
          selected?.incognito && 'is-incognito',
        )}
        ref={rootRef}
      >
        <TabStrip
          tabs={tabs}
          selectedId={selectedId}
          starting={starting}
          onSelect={setSelectedId}
          onClose={closeTab}
          onNew={() => void handleNewTab(false)}
        />

        {selected ? (
          <SessionView
            // Remount per tab so drafts, pick mode, and type buffers never
            // leak across tabs.
            key={selected.session_id}
            host={host}
            onOpenSavedSets={openSavedSets}
            session={selected}
            enabled
            tabId={tabId}
            actionsRef={sessionActionsRef}
            onSessionsRefresh={refresh}
            onNewTab={(incognito) => void handleNewTab(incognito)}
          />
        ) : (
          <section className="br-ui-stage" aria-label="browser workspace">
            <div className="br-ui-hero">
              <Globe size={28} aria-hidden className="br-ui-hero-icon" />
              <h2 className="br-ui-hero-title">
                {loading ? 'Loading tabs…' : 'No open tabs'}
              </h2>
              {loading ? null : (
                <>
                  <p className="br-ui-hero-body">
                    Tabs agents open appear here live. Open one yourself, or ask
                    an agent to call <code>browser::sessions::start</code>.
                  </p>
                  <div className="br-ui-hero-actions">
                    <button
                      type="button"
                      className="br-ui-hero-btn"
                      onClick={() => void handleNewTab(false)}
                      disabled={starting}
                    >
                      <Plus size={16} aria-hidden />
                      {starting ? 'Opening…' : 'New tab'}
                    </button>
                    <button
                      type="button"
                      className="br-ui-hero-btn is-incognito"
                      onClick={() => void handleNewTab(true)}
                      disabled={starting}
                    >
                      <Incognito size={16} aria-hidden />
                      New incognito tab
                    </button>
                  </div>
                </>
              )}
            </div>
          </section>
        )}
      </div>
      <SavedSetsDialog
        host={host}
        open={savedSets.open}
        onOpenChange={(next) =>
          setSavedSets((s) => ({ open: next, key: next ? s.key : null }))
        }
        initialKey={savedSets.key}
      />
    </PageShell>
  )
}
