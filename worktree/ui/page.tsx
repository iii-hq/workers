/**
 * Entry for the worktree worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as worktree/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers one contribution:
 * - src/page/ — the `#/ext/worktree` page: a live repo → worktree → session
 *   topology graph with a per-worktree detail panel, reading the live worker
 *   over `worktree::list` and refreshing on the six lifecycle trigger types.
 *   Read-only on purpose: create / claim / land stay in agent + CLI flows.
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { WorktreesPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'worktree',
    title: 'worktrees',
    render: () => <WorktreesPage host={host} />,
  })
}
