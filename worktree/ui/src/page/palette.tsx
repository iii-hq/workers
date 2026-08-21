/**
 * The worktree worker in the command palette, before its page is even open.
 *
 * A worktrees source answers any query with the live registry —
 * `worktree::list`, the same read useWorktreesLive bootstraps from — each
 * row opening the worktrees page with that worktree selected. Registered
 * from setup, so it exists only while the worker is connected; older
 * consoles without host.palette / host.commands simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { listWorktrees, shortWorktreeId } from './worktree-data'

const WORKTREE_ROWS = 30

export function registerWorktreePalette(host: Host): void {
  host.palette?.registerSource({
    id: 'worktrees',
    title: 'Worktrees',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const worktrees = await listWorktrees(host)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return worktrees
        .filter(
          (worktree) =>
            worktree.branch.toLowerCase().includes(needle) ||
            worktree.worktree_id.toLowerCase().includes(needle) ||
            worktree.path.toLowerCase().includes(needle),
        )
        .slice(0, WORKTREE_ROWS)
        .map((worktree) => ({
          id: worktree.worktree_id,
          title: worktree.branch,
          detail: worktree.path,
          keywords: [shortWorktreeId(worktree.worktree_id)],
          run: () =>
            host.panels?.open({
              pageId: 'worktree',
              context: { worktreeId: worktree.worktree_id },
            }),
        }))
    },
  })

  host.commands?.register('worktree', [
    {
      id: 'open',
      title: 'Open worktrees',
      detail: 'Repo → worktree → session topology',
      keywords: ['repo', 'branch', 'graph'],
      run: () => host.panels?.open({ pageId: 'worktree', context: {} }),
    },
  ])
}
