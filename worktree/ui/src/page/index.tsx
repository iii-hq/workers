/**
 * The worktrees page (#/ext/worktree): the standard page chrome
 * (PageShell/PageHeader from @iii-dev/console-ui) over a live repo →
 * worktree → session topology graph (./WorktreeGraph) and a per-worktree
 * detail column (./WorktreeDetailPanel). Data is `worktree::list
 * {include_status:true}`, refreshed on all six lifecycle trigger types
 * with a poll fallback (see useWorktreesLive). Read-only on purpose:
 * create / claim / land stay in agent + CLI flows.
 *
 * Layout adapts to the width the page HAS (a ResizeObserver on its own
 * body, not a viewport media query — the console can host it in panes of
 * any size). Wide: the graph is the workspace hero with the detail as a
 * fixed-width sidebar column beside it. Under NARROW_BELOW px it becomes
 * a drill-in flow: the graph fills the pane and selecting a worktree
 * swaps it for the detail panel with a ← back button.
 *
 * The host only mounts this page when the worktree worker is connected, so
 * there is no presence gate here; a failed `worktree::list` surfaces as the
 * "failed to load worktrees" alert.
 */

import {
  Button,
  EmptyState,
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useCallback, useMemo, useRef, useState } from 'react'
import { AlertCircle, GitBranch, type IconProps, RefreshCw } from './icons'
import { useWorktreesLive } from './useWorktreesLive'
import { cn } from './worktree-data'
import { WorktreeDetailPanel } from './WorktreeDetailPanel'
import { WorktreeGraph } from './WorktreeGraph'

/** Container width (px) below which the page collapses to the drill-in
 * graph ⇄ detail flow: the detail column (300) plus a usable slice of the
 * graph's repo + worktree columns. */
const NARROW_BELOW = 860

/** Observe the page body's own width. Returns a callback ref to put on
 * the body plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none) are ignored so a hidden page keeps its last
 * real layout. */
function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}

const EmptyIcon = (p: IconProps) => <GitBranch size={26} {...p} />

export function WorktreesPage({
  host,
  panelSide = 'left',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const { worktrees, loading, error, live, refresh } = useWorktreesLive(host)

  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected = useMemo(
    () => worktrees.find((w) => w.worktree_id === selectedId) ?? null,
    [worktrees, selectedId],
  )

  const [bodyRef, narrow] = useContainerNarrow(NARROW_BELOW)

  const countLabel = loading ? '...' : String(worktrees.length)

  // Narrow: one pane at a time — selecting a worktree drills into its
  // detail, back deselects. A worktree removed out from under the
  // selection (landed / removed event) drops `selected` and falls back
  // to the graph on its own.
  const showCanvas = !narrow || selected === null

  return (
    <PageShell className="wt-ui-shell">
      <PageHeader
        icon={<GitBranch />}
        title="worktrees"
        description={`${countLabel} managed`}
        actions={
          <>
            <span
              className="wt-ui-liveness"
              title={
                live
                  ? 'updates arrive on the worktree lifecycle triggers'
                  : 'live bindings unavailable; refreshing on a timer'
              }
            >
              <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
              {live ? 'live' : 'polling'}
            </span>
            <Button variant="ghost" size="sm" onClick={refresh} disabled={loading}>
              <RefreshCw
                size={13}
                className={cn('wt-ui-refresh-icon', loading && 'spin')}
                aria-hidden
              />
              refresh
            </Button>
          </>
        }
        onClose={onRequestClose}
      />

      <div
        className={cn('wt-ui-body', narrow && 'narrow', panelSide === 'right' && 'right')}
        ref={bodyRef}
      >
        {showCanvas ? (
          <div className="wt-ui-canvas">
            {error ? (
              <StatusPanel
                variant="alert"
                icon={<AlertCircle size={18} />}
                headline="failed to load worktrees"
                detail={error}
              />
            ) : loading && worktrees.length === 0 ? (
              <div className="wt-ui-skel" aria-hidden>
                <span className="bar w40" />
                <span className="bar w75" />
                <span className="bar w60" />
              </div>
            ) : worktrees.length === 0 ? (
              <EmptyState
                icon={EmptyIcon}
                title="no worktrees yet"
                description="worktrees are isolated checkouts for parallel agent work: each agent gets its own branch and directory, then lands finished work back with worktree::land. create one from a chat's directory picker, ask an agent to call worktree::create, or run it from the CLI: iii trigger worktree::create"
              />
            ) : (
              <WorktreeGraph
                worktrees={worktrees}
                selectedId={selectedId}
                onSelect={(id) => setSelectedId((cur) => (cur === id ? null : id))}
              />
            )}
          </div>
        ) : null}
        {selected ? (
          <WorktreeDetailPanel
            worktree={selected}
            narrow={narrow}
            onClose={() => setSelectedId(null)}
          />
        ) : null}
      </div>
    </PageShell>
  )
}
