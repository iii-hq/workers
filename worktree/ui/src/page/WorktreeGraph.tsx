import { StatusDot } from '@iii-dev/console-ui'
import { useMemo } from 'react'
import { FolderGit2, GitBranch, GitMerge } from './icons'
import { layoutWorktreeGraph } from './layout'
import {
  cn,
  integrationLabel,
  lifecycleTone,
  lifecycleToneClass,
  shortWorktreeId,
  type WorktreeInfo,
  worktreeIndicators,
} from './worktree-data'

/**
 * Hand-rolled graph surface: one absolutely-positioned HTML node per repo
 * and worktree on top of a single SVG carrying the orthogonal edges, all
 * sized by the pure layout in `layout.ts`. No graph dependency; the
 * schematic feel comes from 1px rules, elbow edges, and the lifecycle dot.
 * Ported from the console page — the layout math is verbatim; Tailwind
 * utilities became scoped `wt-*` classes (see styles.css) and lucide icons
 * became the inline set in icons.tsx.
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
      className="wt-graph"
      style={{ width: layout.width, height: layout.height }}
    >
      <svg
        aria-hidden="true"
        role="presentation"
        className="wt-graph-svg"
        width={layout.width}
        height={layout.height}
      >
        {layout.edges.map((edge) => (
          <g key={edge.worktreeId}>
            <path
              d={`M ${edge.from.x} ${edge.from.y} H ${edge.midX} V ${edge.to.y} H ${edge.to.x}`}
              className={cn(
                'wt-edge',
                selectedId === edge.worktreeId && 'selected',
              )}
              strokeWidth={1}
            />
            {edge.label ? (
              <text
                x={edge.midX}
                y={edge.to.y - 5}
                textAnchor="middle"
                className="wt-edge-label"
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
          className="wt-repo"
          style={{ left: repo.x, top: repo.y, width: repo.w, height: repo.h }}
          title={repo.repoPath}
        >
          <span className="wt-repo-name">
            <FolderGit2 size={13} className="wt-icon-faint" aria-hidden />
            <span className="wt-truncate">{repo.label}</span>
          </span>
          <span className="wt-repo-path">{repo.repoPath}</span>
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
            className={cn('wt-node', selected && 'selected')}
            style={{ left: node.x, top: node.y, width: node.w, height: node.h }}
          >
            <span className="wt-node-branch">
              <GitBranch size={12} className="wt-icon-faint" aria-hidden />
              <span className="wt-truncate wt-ink">{wt.branch}</span>
              <span className="wt-node-id">{shortWorktreeId(wt.worktree_id)}</span>
              {dirty ? (
                <span className="wt-dirty" title="uncommitted changes">
                  *
                </span>
              ) : null}
              {ahead > 0 ? (
                <span className="wt-ahead" title={`${ahead} commit(s) ahead of base`}>
                  +{ahead}
                </span>
              ) : null}
              {wt.status?.integrated ? (
                <span className="wt-icon-ghost" title={integrationLabel(wt.status)}>
                  <GitMerge size={11} aria-hidden />
                </span>
              ) : null}
            </span>
            <span className="wt-node-meta">
              <span className={cn('wt-lifecycle', lifecycleToneClass[tone])}>
                <StatusDot tone={tone} pulse={wt.lifecycle === 'landing'} />
                {wt.lifecycle}
              </span>
              {wt.session_id ? (
                <span className="wt-session" title={`claimed by ${wt.session_id}`}>
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
