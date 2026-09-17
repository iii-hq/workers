/**
 * The github page shell (page `github`): the standard page chrome
 * (PageShell/PageHeader from @iii-dev/console-ui) with a graph | activity
 * segmented control in the header's middle slot — one header, no stacked
 * top bars.
 *
 * - graph (default, the hero): a live commit DAG of the working repo —
 *   colored lanes, merge routing, ref labels — refreshing as the agent
 *   works (GitGraph).
 * - activity: the live feed of `github::called` events, one row per
 *   finished call (ActivityFeed, from ./index).
 *
 * Only the selected view is mounted, so each view owns its own
 * `github::called` subscription only while visible — the two never run at
 * once (the same lifecycle the previous Radix Tabs gave us). The selected
 * view persists per workspace tab (`tabId` namespaces the localStorage
 * key; workspace tabs survive reloads).
 */

import {
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  SegmentedControl,
} from '@iii-dev/console-ui'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { useEffect, useRef } from 'react'
import { GitGraph } from './GitGraph'
import { ActivityFeed } from './index'

type View = 'graph' | 'activity'

const VIEWS: { value: View; label: string }[] = [
  { value: 'graph', label: 'Graph' },
  { value: 'activity', label: 'Activity' },
]

/** The github mark (lucide `github` geometry) — the page's identity glyph.
 *  lucide-react ships no brand marks, so this one stays hand-drawn. */
function GithubMark() {
  return (
    // Brand mark absent from lucide-react. lint-allow no-inline-svg
    <svg
      width={16}
      height={16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.51 2-5-2-7-2" />
    </svg>
  )
}

export function GithubPage({
  host,
  tabId = '',
  onRequestClose,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const storageKey = `github-ui:${tabId || 'page'}:view`
  const [stored, setView] = usePaneState<View>(storageKey, 'graph')
  const view: View = stored === 'activity' ? 'activity' : 'graph'

  // Set by whichever view is mounted (graph or activity), so one set of page
  // commands reaches either one without lifting their state up here.
  const liveRef = useRef<(() => void) | null>(null)
  const refreshRef = useRef<(() => void) | null>(null)
  const closeDetailRef = useRef<(() => void) | null>(null)

  useEffect(
    () =>
      commands?.register([
        {
          id: 'toggle-live',
          title: 'Toggle live updates',
          detail: 'Pause or resume this view',
          keywords: ['pause', 'resume'],
          enabled: () => liveRef.current !== null,
          run: () => liveRef.current?.(),
        },
        {
          id: 'refresh',
          title: 'Refresh the commit graph',
          keywords: ['reload', 'git log'],
          enabled: () => refreshRef.current !== null,
          run: () => refreshRef.current?.(),
        },
        {
          id: 'close-detail',
          title: 'Close the open detail',
          keywords: ['back'],
          shortcut: 'Escape',
          enabled: () => closeDetailRef.current !== null,
          run: () => closeDetailRef.current?.(),
        },
      ]),
    [commands],
  )

  return (
    <PageShell className="gh-ui-shell">
      <PageHeader
        icon={<GithubMark />}
        title="GitHub"
        description="The working repository and worker calls, live"
        onClose={onRequestClose}
      >
        <SegmentedControl<View>
          value={view}
          onChange={setView}
          options={VIEWS}
          className="gh-ui-tabs"
          aria-label="Graph or activity view"
        />
      </PageHeader>
      {view === 'graph' ? (
        <GitGraph
          host={host}
          liveRef={liveRef}
          refreshRef={refreshRef}
          closeDetailRef={closeDetailRef}
        />
      ) : (
        <ActivityFeed
          host={host}
          liveRef={liveRef}
          closeDetailRef={closeDetailRef}
        />
      )}
    </PageShell>
  )
}
