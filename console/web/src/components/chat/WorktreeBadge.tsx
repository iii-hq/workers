import { GitBranch } from 'lucide-react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import {
  lifecycleTone,
  lifecycleToneClass,
  shortWorktreeId,
  type WorktreeInfo,
  worktreeIndicators,
} from '@/lib/worktrees'

/**
 * Working-directory badge for a managed worktree: branch, short id, dirty
 * and ahead indicators, and a lifecycle dot (accent while landing, alert
 * when land-blocked — text color plus a dot, never a status fill, per the
 * design system). The raw path stays reachable as the tooltip; ChatView
 * falls back to the plain path chip when the dir is not a managed worktree.
 */

interface WorktreeBadgeProps {
  worktree: WorktreeInfo
  className?: string
}

export function WorktreeBadge({ worktree, className }: WorktreeBadgeProps) {
  const tone = lifecycleTone(worktree.lifecycle)
  const { dirty, ahead } = worktreeIndicators(worktree.status)
  const showLifecycle = worktree.lifecycle !== 'active'
  return (
    <span
      className={cn('flex min-w-0 items-center gap-1.5 font-mono', className)}
      title={`${worktree.path} — worktree ${worktree.worktree_id} on ${worktree.branch}`}
    >
      <GitBranch size={12} className="shrink-0 text-ink-faint" aria-hidden />
      <span className="truncate text-ink">{worktree.branch}</span>
      <span className="shrink-0 text-ink-ghost tabular-nums">
        {shortWorktreeId(worktree.worktree_id)}
      </span>
      {dirty ? (
        <span className="shrink-0 text-warn" title="uncommitted changes">
          *
        </span>
      ) : null}
      {ahead > 0 ? (
        <span
          className="shrink-0 text-ink-faint tabular-nums"
          title={`${ahead} commit(s) ahead of base`}
        >
          +{ahead}
        </span>
      ) : null}
      {showLifecycle ? (
        <span
          className={cn(
            'flex shrink-0 items-center gap-1 lowercase',
            lifecycleToneClass[tone],
          )}
        >
          <StatusDot tone={tone} pulse={worktree.lifecycle === 'landing'} />
          {worktree.lifecycle}
        </span>
      ) : null}
    </span>
  )
}
