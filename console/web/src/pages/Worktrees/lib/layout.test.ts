import { describe, expect, it } from 'vitest'
import type { WorktreeInfo } from '@/lib/worktrees'
import { GRAPH, layoutWorktreeGraph, repoLabel } from './layout'

function wt(
  overrides: Partial<WorktreeInfo> & { worktree_id: string },
): WorktreeInfo {
  return {
    repo_path: '/home/dev/app',
    repo_key: '/home/dev/app/.git',
    path: `/wts/${overrides.worktree_id}`,
    branch: `iii/${overrides.worktree_id}`,
    lifecycle: 'active',
    ...overrides,
  }
}

describe('layoutWorktreeGraph', () => {
  it('returns an empty layout for no worktrees', () => {
    const layout = layoutWorktreeGraph([])
    expect(layout).toEqual({
      width: 0,
      height: 0,
      repos: [],
      nodes: [],
      edges: [],
    })
  })

  it('groups by repo key and sorts repos deterministically', () => {
    const layout = layoutWorktreeGraph([
      wt({
        worktree_id: 'wt_b1',
        repo_key: '/home/dev/zeta/.git',
        repo_path: '/home/dev/zeta',
      }),
      wt({ worktree_id: 'wt_a1' }),
      wt({ worktree_id: 'wt_a2' }),
    ])
    expect(layout.repos.map((r) => r.key)).toEqual([
      '/home/dev/app/.git',
      '/home/dev/zeta/.git',
    ])
    expect(layout.repos.map((r) => r.label)).toEqual(['app', 'zeta'])
    expect(layout.nodes).toHaveLength(3)
    expect(layout.edges).toHaveLength(3)
  })

  it('orders worktrees within a repo by created_at then id', () => {
    const layout = layoutWorktreeGraph([
      wt({ worktree_id: 'wt_late', created_at: 300 }),
      wt({ worktree_id: 'wt_zz', created_at: 100 }),
      wt({ worktree_id: 'wt_aa', created_at: 100 }),
    ])
    expect(layout.nodes.map((n) => n.worktree.worktree_id)).toEqual([
      'wt_aa',
      'wt_zz',
      'wt_late',
    ])
    // Stacked top-down in that order.
    const ys = layout.nodes.map((n) => n.y)
    expect(ys[0]).toBeLessThan(ys[1])
    expect(ys[1]).toBeLessThan(ys[2])
  })

  it('computes stable column and stack positions', () => {
    const layout = layoutWorktreeGraph([
      wt({ worktree_id: 'wt_a1', created_at: 1 }),
      wt({ worktree_id: 'wt_a2', created_at: 2 }),
    ])
    const { pad, repoW, repoH, nodeW, nodeH, hGap, vGap } = GRAPH
    const nodeX = pad + repoW + hGap
    expect(layout.repos[0].x).toBe(pad)
    expect(layout.nodes.map((n) => n.x)).toEqual([nodeX, nodeX])
    expect(layout.nodes[0].y).toBe(pad)
    expect(layout.nodes[1].y).toBe(pad + nodeH + vGap)
    // Repo node is vertically centered on its group.
    const groupH = 2 * nodeH + vGap
    expect(layout.repos[0].y).toBe(pad + groupH / 2 - repoH / 2)
    expect(layout.width).toBe(pad * 2 + repoW + hGap + nodeW)
    expect(layout.height).toBe(pad * 2 + groupH)
  })

  it('separates repo groups vertically', () => {
    const layout = layoutWorktreeGraph([
      wt({ worktree_id: 'wt_a1' }),
      wt({
        worktree_id: 'wt_b1',
        repo_key: '/home/dev/zeta/.git',
        repo_path: '/home/dev/zeta',
      }),
    ])
    const { pad, repoH, nodeH, groupGap } = GRAPH
    const firstGroupH = Math.max(nodeH, repoH)
    expect(layout.nodes[1].y).toBe(pad + firstGroupH + groupGap)
  })

  it('draws edges from repo right edge to worktree left edge', () => {
    const layout = layoutWorktreeGraph([wt({ worktree_id: 'wt_a1' })])
    const { pad, repoW, hGap } = GRAPH
    const [edge] = layout.edges
    const [repo] = layout.repos
    const [node] = layout.nodes
    expect(edge.from).toEqual({ x: pad + repoW, y: repo.y + repo.h / 2 })
    expect(edge.to).toEqual({ x: node.x, y: node.y + node.h / 2 })
    expect(edge.midX).toBe(pad + repoW + hGap / 2)
  })

  it('labels edges with base_ref only when not the default HEAD', () => {
    const layout = layoutWorktreeGraph([
      wt({ worktree_id: 'wt_a1', base_ref: 'HEAD' }),
      wt({ worktree_id: 'wt_a2', base_ref: 'release/1.2' }),
      wt({ worktree_id: 'wt_a3' }),
    ])
    const byId = new Map(layout.edges.map((e) => [e.worktreeId, e.label]))
    expect(byId.get('wt_a1')).toBeNull()
    expect(byId.get('wt_a2')).toBe('release/1.2')
    expect(byId.get('wt_a3')).toBeNull()
  })

  it('falls back to repo_path grouping when repo_key is absent', () => {
    const layout = layoutWorktreeGraph([
      wt({ worktree_id: 'wt_a1', repo_key: undefined }),
      wt({ worktree_id: 'wt_a2', repo_key: undefined }),
    ])
    expect(layout.repos).toHaveLength(1)
    expect(layout.repos[0].key).toBe('/home/dev/app')
  })
})

describe('repoLabel', () => {
  it('uses the repo directory name', () => {
    expect(repoLabel('/home/dev/app')).toBe('app')
    expect(repoLabel('/srv/repos/api/')).toBe('api')
    expect(repoLabel('/')).toBe('/')
  })
})
