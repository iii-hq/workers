import { AlertCircle, GitBranch, RefreshCw } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import {
  isWorktreeAvailable,
  useWorktreeStatus,
} from '@/hooks/use-worktree-status'
import { useConversationsCtx } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import { WorktreeDetailPanel } from './components/WorktreeDetailPanel'
import { WorktreeGraph } from './components/WorktreeGraph'
import { useWorktreesLive } from './hooks/useWorktreesLive'

/**
 * Live worktree graph: repo nodes -> worktree nodes -> owning sessions, so
 * a user running parallel worktrees sees the tree while agents work. Data
 * is `worktree::list {include_status:true}`, refreshed on all six lifecycle
 * trigger types (poll fallback while bindings are unavailable). The nav
 * entry is gated on worker presence; a direct hash hit with the worker
 * absent lands on the install notice below.
 */

export function Worktrees() {
  const { backend } = useConversationsCtx()
  const status = useWorktreeStatus(backend.id === 'real')
  const available = isWorktreeAvailable(status)

  const { worktrees, loading, error, live, refresh } =
    useWorktreesLive(available)

  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected = useMemo(
    () => worktrees.find((w) => w.worktree_id === selectedId) ?? null,
    [worktrees, selectedId],
  )

  const countLabel = loading ? '...' : String(worktrees.length)

  return (
    <main
      className="flex-1 flex flex-col min-h-0 overflow-hidden"
      aria-label="worktrees"
    >
      <header className="shrink-0 px-4 sm:px-6 lg:px-8 py-4 border-b border-rule flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink lowercase">
            worktrees
          </h1>
          <p className="font-mono text-[12px] text-ink-faint mt-0.5 lowercase">
            {available ? `${countLabel} managed` : 'worker not connected'}
          </p>
        </div>
        {available ? (
          <div className="flex items-center gap-3">
            <span
              className="flex items-center gap-1.5 font-mono text-[11px] lowercase text-ink-faint"
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
              className="gap-1.5"
            >
              <RefreshCw
                className={cn('w-3.5 h-3.5', loading && 'animate-spin')}
                aria-hidden
              />
              refresh
            </Button>
          </div>
        ) : null}
      </header>

      <div className="flex-1 flex min-h-0">
        <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4">
          {!available ? (
            status.loading ? (
              <p className="font-mono text-[12px] lowercase text-ink-ghost">
                checking for the worktree worker...
              </p>
            ) : (
              <StatusPanel
                variant="info"
                icon={<AlertCircle className="w-full h-full" />}
                headline="worktree worker not installed"
                detail="this page needs the optional worktree worker. run: iii worker add worktree"
              />
            )
          ) : error ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle className="w-full h-full" />}
              headline="failed to load worktrees"
              detail={error}
            />
          ) : !loading && worktrees.length === 0 ? (
            <EmptyState
              icon={GitBranch}
              title="no worktrees yet"
              description="worktrees are isolated checkouts for parallel agent work: each agent gets its own branch and directory, then lands finished work back with worktree::land. create one from a chat's directory picker, ask an agent to call worktree::create, or run it from the CLI: iii trigger worktree::create"
            />
          ) : (
            <WorktreeGraph
              worktrees={worktrees}
              selectedId={selectedId}
              onSelect={(id) =>
                setSelectedId((cur) => (cur === id ? null : id))
              }
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
    </main>
  )
}
