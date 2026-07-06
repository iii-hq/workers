import type { WorktreeInfo } from '@/lib/worktrees'

/**
 * Pure layered layout for the worktree graph: one repo node column on the
 * left, its worktrees stacked in a column on the right, groups stacked
 * vertically. Deterministic — grouped by repo key, repos sorted by key,
 * worktrees sorted by created_at then id — so re-renders and tests always
 * see identical positions. All geometry in px; rendering maps 1:1.
 */

export const GRAPH = {
  pad: 24,
  repoW: 200,
  repoH: 48,
  nodeW: 248,
  nodeH: 56,
  /** Horizontal run between the repo column and the worktree column. */
  hGap: 96,
  /** Vertical gap between sibling worktree nodes. */
  vGap: 12,
  /** Vertical gap between repo groups. */
  groupGap: 40,
} as const

export interface RepoNodeLayout {
  /** Stable grouping key (repo_key when exposed, else repo_path). */
  key: string
  repoPath: string
  /** Display label: the repo directory name. */
  label: string
  x: number
  y: number
  w: number
  h: number
}

export interface WorktreeNodeLayout {
  worktree: WorktreeInfo
  x: number
  y: number
  w: number
  h: number
}

export interface EdgeLayout {
  worktreeId: string
  /** Repo-node attachment point (right edge center). */
  from: { x: number; y: number }
  /** Worktree-node attachment point (left edge center). */
  to: { x: number; y: number }
  /** X of the vertical elbow segment. */
  midX: number
  /** base_ref, only when it is not the default HEAD. */
  label: string | null
}

export interface WorktreeGraphLayout {
  width: number
  height: number
  repos: RepoNodeLayout[]
  nodes: WorktreeNodeLayout[]
  edges: EdgeLayout[]
}

export function repoLabel(repoPath: string): string {
  const parts = repoPath.split('/').filter(Boolean)
  return parts.length ? parts[parts.length - 1] : repoPath
}

function groupKey(wt: WorktreeInfo): string {
  return wt.repo_key ?? wt.repo_path
}

function byCreation(a: WorktreeInfo, b: WorktreeInfo): number {
  const at = a.created_at ?? 0
  const bt = b.created_at ?? 0
  if (at !== bt) return at - bt
  return a.worktree_id.localeCompare(b.worktree_id)
}

export function layoutWorktreeGraph(
  worktrees: WorktreeInfo[],
): WorktreeGraphLayout {
  if (worktrees.length === 0) {
    return { width: 0, height: 0, repos: [], nodes: [], edges: [] }
  }

  const groups = new Map<string, WorktreeInfo[]>()
  for (const wt of worktrees) {
    const key = groupKey(wt)
    const members = groups.get(key)
    if (members) members.push(wt)
    else groups.set(key, [wt])
  }
  const keys = [...groups.keys()].sort((a, b) => a.localeCompare(b))

  const { pad, repoW, repoH, nodeW, nodeH, hGap, vGap, groupGap } = GRAPH
  const nodeX = pad + repoW + hGap
  const midX = pad + repoW + hGap / 2

  const repos: RepoNodeLayout[] = []
  const nodes: WorktreeNodeLayout[] = []
  const edges: EdgeLayout[] = []

  let y = pad
  for (const key of keys) {
    const members = groups.get(key)
    if (!members) continue
    members.sort(byCreation)

    const groupH = Math.max(
      members.length * nodeH + (members.length - 1) * vGap,
      repoH,
    )
    const repoY = y + groupH / 2 - repoH / 2
    repos.push({
      key,
      repoPath: members[0].repo_path,
      label: repoLabel(members[0].repo_path),
      x: pad,
      y: repoY,
      w: repoW,
      h: repoH,
    })

    let memberY = y
    for (const wt of members) {
      nodes.push({ worktree: wt, x: nodeX, y: memberY, w: nodeW, h: nodeH })
      edges.push({
        worktreeId: wt.worktree_id,
        from: { x: pad + repoW, y: repoY + repoH / 2 },
        to: { x: nodeX, y: memberY + nodeH / 2 },
        midX,
        label: wt.base_ref && wt.base_ref !== 'HEAD' ? wt.base_ref : null,
      })
      memberY += nodeH + vGap
    }

    y += groupH + groupGap
  }

  return {
    width: pad * 2 + repoW + hGap + nodeW,
    height: y - groupGap + pad,
    repos,
    nodes,
    edges,
  }
}
