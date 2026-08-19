/**
 * The github page shell (#/ext/github): the standard page chrome
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
import { useState } from 'react'
import { GitGraph } from './GitGraph'
import { GithubIcon } from './icons'
import { ActivityFeed } from './index'

type View = 'graph' | 'activity'

const VIEWS: { value: View; label: string }[] = [
  { value: 'graph', label: 'Graph' },
  { value: 'activity', label: 'Activity' },
]

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeStored(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value)
  } catch {
    /* private mode / quota — persistence is best-effort */
  }
}

export function GithubPage({
  host,
  tabId = '',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const storageKey = `github-ui:${tabId || 'page'}:view`
  const [view, setViewState] = useState<View>(() =>
    readStored(storageKey) === 'activity' ? 'activity' : 'graph',
  )
  const setView = (next: View) => {
    setViewState(next)
    writeStored(storageKey, next)
  }

  return (
    <PageShell className="gh-ui-shell">
      <PageHeader
        icon={<GithubIcon />}
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
        <GitGraph host={host} />
      ) : (
        <ActivityFeed host={host} />
      )}
    </PageShell>
  )
}
