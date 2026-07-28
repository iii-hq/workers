/**
 * The worktrees page (#/ext/worktree): a live repo → worktree → session
 * topology so a user running parallel worktrees sees the tree while agents
 * work. Data is `worktree::list {include_status:true}`, refreshed on all six
 * lifecycle trigger types with a poll fallback (see useWorktreesLive).
 * Read-only on purpose: create / claim / land stay in agent + CLI flows.
 *
 * The host only mounts this page when the worktree worker is connected, so
 * there is no presence gate here; a failed `worktree::list` surfaces as the
 * "failed to load worktrees" alert.
 */

import { Button, EmptyState, type Host, StatusDot, StatusPanel } from '@iii-dev/console-ui'
import { useMemo, useState } from 'react'
import { AlertCircle, GitBranch, type IconProps, RefreshCw } from './icons'
import { useWorktreesLive } from './useWorktreesLive'
import { cn } from './worktree-data'
import { WorktreeDetailPanel } from './WorktreeDetailPanel'
import { WorktreeGraph } from './WorktreeGraph'

const EmptyIcon = (p: IconProps) => <GitBranch size={26} {...p} />

export function WorktreesPage({ host }: { host: Host }) {
  const { worktrees, loading, error, live, refresh } = useWorktreesLive(host)

  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected = useMemo(
    () => worktrees.find((w) => w.worktree_id === selectedId) ?? null,
    [worktrees, selectedId],
  )

  const countLabel = loading ? '...' : String(worktrees.length)

  return (
    <div className="wt-page">
      <header className="wt-head">
        <div>
          <div className="wt-title">worktrees</div>
          <div className="wt-sub">{countLabel} managed</div>
        </div>
        <div className="wt-head-actions">
          <span
            className="wt-liveness"
            title={
              live
                ? 'updates arrive on the worktree lifecycle triggers'
                : 'live bindings unavailable; refreshing on a timer'
            }
          >
            <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
            {live ? 'live' : 'polling'}
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={refresh}
            disabled={loading}
          >
            <RefreshCw
              size={13}
              className={cn('wt-refresh-icon', loading && 'spin')}
              aria-hidden
            />
            refresh
          </Button>
        </div>
      </header>

      <div className="wt-body">
        <div className="wt-canvas">
          {error ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle size={18} />}
              headline="failed to load worktrees"
              detail={error}
            />
          ) : !loading && worktrees.length === 0 ? (
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
        {selected ? (
          <WorktreeDetailPanel
            worktree={selected}
            onClose={() => setSelectedId(null)}
          />
        ) : null}
      </div>
    </div>
  )
}
