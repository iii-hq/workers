/**
 * Entry for the github worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as github/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers one contribution:
 * - src/page/ — the `#/ext/github` browser: owner/name input plus the
 *   pull-requests / issues / runs / releases / search panels, each reading
 *   the live github worker over `github::*` and unwrapping its `{ value }`
 *   envelope. Read-only on purpose: mutations stay in agent flows behind
 *   the approval gate.
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { GithubPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'github',
    title: 'github',
    render: () => <GithubPage host={host} />,
  })
}
