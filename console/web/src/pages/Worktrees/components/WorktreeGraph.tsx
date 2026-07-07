import { FolderGit2, GitBranch, GitMerge } from 'lucide-react'
import { useMemo } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import {
  integrationLabel,
  lifecycleTone,
  lifecycleToneClass,
  shortWorktreeId,
  type WorktreeInfo,
  worktreeIndicators,
} from '@/lib/worktrees'
import { layoutWorktreeGraph } from '../lib/layout'

/**
 * Hand-rolled graph surface: one absolutely-positioned HTML node per repo
 * and worktree on top of a single SVG carrying the orthogonal edges, all
 * sized by the pure layout in `lib/layout.ts`. No graph dependency; the
 * schematic feel comes from 1px rules, elbow edges, and the lifecycle dot.
 */

interface WorktreeGraphProps {
  worktrees: WorktreeInfo[]
  selectedId: string | null
  onSelect: (worktreeId: string) => void
}

export function WorktreeGraph({
  worktrees,
  selectedId,
  onSelect,
}: WorktreeGraphProps) {
  const layout = useMemo(() => layoutWorktreeGraph(worktrees), [worktrees])
  if (layout.nodes.length === 0) return null

  return (
    <div
      className="relative"
      style={{ width: layout.width, height: layout.height }}
    >
      <svg
        aria-hidden="true"
        role="presentation"
        className="absolute inset-0"
        width={layout.width}
        height={layout.height}
      >
        {layout.edges.map((edge) => (
          <g key={edge.worktreeId}>
            <path
              d={`M ${edge.from.x} ${edge.from.y} H ${edge.midX} V ${edge.to.y} H ${edge.to.x}`}
              className={cn(
                'fill-none',
                selectedId === edge.worktreeId
                  ? 'stroke-accent'
                  : 'stroke-rule',
              )}
              strokeWidth={1}
            />
            {edge.label ? (
              <text
                x={edge.midX}
                y={edge.to.y - 5}
                textAnchor="middle"
                className="fill-ink-ghost font-mono text-[10px] lowercase"
              >
                {edge.label}
              </text>
            ) : null}
          </g>
        ))}
      </svg>

      {layout.repos.map((repo) => (
        <div
          key={repo.key}
          className="absolute flex flex-col justify-center gap-0.5 border border-rule bg-panel px-3"
          style={{ left: repo.x, top: repo.y, width: repo.w, height: repo.h }}
          title={repo.repoPath}
        >
          <span className="flex items-center gap-1.5 font-mono text-[12px] font-semibold lowercase text-ink">
            <FolderGit2
              size={13}
              className="shrink-0 text-ink-faint"
              aria-hidden
            />
            <span className="truncate">{repo.label}</span>
          </span>
          <span className="truncate font-mono text-[10px] text-ink-ghost">
            {repo.repoPath}
          </span>
        </div>
      ))}

      {layout.nodes.map((node) => {
        const wt = node.worktree
        const tone = lifecycleTone(wt.lifecycle)
        const { dirty, ahead } = worktreeIndicators(wt.status)
        const selected = selectedId === wt.worktree_id
        return (
          <button
            key={wt.worktree_id}
            type="button"
            onClick={() => onSelect(wt.worktree_id)}
            aria-pressed={selected}
            title={
              wt.base_ref && wt.base_ref !== 'HEAD'
                ? `${wt.path} — based on ${wt.base_ref}`
                : wt.path
            }
            className={cn(
              'absolute flex flex-col justify-center gap-1 border px-3 text-left transition-colors',
              selected
                ? 'border-rule border-l-2 border-l-accent bg-panel'
                : 'border-rule bg-bg hover:bg-panel',
            )}
            style={{ left: node.x, top: node.y, width: node.w, height: node.h }}
          >
            <span className="flex min-w-0 items-center gap-1.5 font-mono text-[12px]">
              <GitBranch
                size={12}
                className="shrink-0 text-ink-faint"
                aria-hidden
              />
              <span className="truncate text-ink">{wt.branch}</span>
              <span className="shrink-0 text-[10px] text-ink-ghost tabular-nums">
                {shortWorktreeId(wt.worktree_id)}
              </span>
              {dirty ? (
                <span
                  className="shrink-0 text-warn"
                  title="uncommitted changes"
                >
                  *
                </span>
              ) : null}
              {ahead > 0 ? (
                <span
                  className="shrink-0 text-[10px] text-ink-faint tabular-nums"
                  title={`${ahead} commit(s) ahead of base`}
                >
                  +{ahead}
                </span>
              ) : null}
              {wt.status?.integrated ? (
                <span
                  className="shrink-0 text-ink-ghost"
                  title={integrationLabel(wt.status)}
                >
                  <GitMerge size={11} aria-hidden />
                </span>
              ) : null}
            </span>
            <span className="flex min-w-0 items-center gap-2 font-mono text-[10px] lowercase">
              <span
                className={cn(
                  'flex items-center gap-1',
                  lifecycleToneClass[tone],
                )}
              >
                <StatusDot tone={tone} pulse={wt.lifecycle === 'landing'} />
                {wt.lifecycle}
              </span>
              {wt.session_id ? (
                <span
                  className="min-w-0 truncate border border-rule-2 px-1 text-ink-ghost"
                  title={`claimed by ${wt.session_id}`}
                >
                  {wt.session_id}
                </span>
              ) : null}
            </span>
          </button>
        )
      })}
    </div>
  )
}
