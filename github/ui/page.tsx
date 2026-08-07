/**
 * Entry for the github worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as github/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers one contribution:
 * - src/page/GithubPage — the `#/ext/github` page: the standard page chrome
 *   with a graph | activity switch over the worker's live bus. GRAPH
 *   (default, the hero) draws the working repo's commit DAG from a single
 *   `shell::exec` `git log --all`, refreshing as the agent works. ACTIVITY
 *   is the live feed — a tab-scoped subscription to `github::called`
 *   rendering each github call as it finishes. Both are read-only observers
 *   of the bus (the graph additionally runs `git log` / `git show` via the
 *   shell worker; it never writes).
 *
 * The page render receives the host's `PageRenderProps` (tabId for per-tab
 * persistence, onRequestClose for the header's ✕). Registrations go through
 * `host` so the loader disposes them on hot reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { GithubPage } from './src/page/GithubPage'

export default function setup(host: Host) {
  host.pages.register({
    id: 'github',
    title: 'github',
    render: (props) => <GithubPage host={host} {...props} />,
  })
}
