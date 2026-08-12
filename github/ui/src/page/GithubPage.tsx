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

import { type Host, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { useState } from 'react'
import { GitGraph } from './GitGraph'
import { GithubIcon } from './icons'
import { ActivityFeed } from './index'

type View = 'graph' | 'activity'

const VIEWS: { value: View; label: string }[] = [
  { value: 'graph', label: 'graph' },
  { value: 'activity', label: 'activity' },
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
  const [view, setViewState] = useState<View>(() => (readStored(storageKey) === 'activity' ? 'activity' : 'graph'))
  const setView = (next: View) => {
    setViewState(next)
    writeStored(storageKey, next)
  }

  return (
    <PageShell className="gh-ui-shell">
      <PageHeader
        icon={<GithubIcon />}
        title="github"
        description="the working repo & worker calls, live"
        onClose={onRequestClose}
      >
        {/* biome-ignore lint/a11y/useSemanticElements: segmented control of buttons; fieldset chrome (min-content sizing) breaks the header row */}
        <div className="gh-ui-seg" role="group" aria-label="graph or activity view">
          {VIEWS.map((v) => (
            <button
              key={v.value}
              type="button"
              className={`gh-ui-seg-btn${view === v.value ? ' active' : ''}`}
              aria-pressed={view === v.value}
              onClick={() => setView(v.value)}
            >
              {v.label}
            </button>
          ))}
        </div>
      </PageHeader>
      {view === 'graph' ? <GitGraph host={host} /> : <ActivityFeed host={host} />}
    </PageShell>
  )
}
