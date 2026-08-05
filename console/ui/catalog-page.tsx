/**
 * Entry for the console worker's engine-catalogue pages — compiled by
 * esbuild (react + @iii-dev/console-ui external) into dist/catalog-page.js
 * and served over the `console:script` trigger (see src/ui.rs). The
 * stylesheet is its own asset: styles.css ships over `console:style` as
 * console/styles.css.
 *
 * Three contributions, all reading engine-level data no single worker owns:
 *
 * - src/catalog/FunctionsPage — every registered function, its schemas, an
 *                               invoke panel, and its live call feed
 *                               (#/ext/functions)
 * - src/catalog/TriggersPage  — trigger types and their live bindings, each
 *                               with its family's real fire path
 *                               (#/ext/triggers)
 * - src/catalog/WorkersPage   — the connected fleet, with each worker's
 *                               functions, trigger types, bindings, and
 *                               process metrics (#/ext/fleet)
 *
 * All three run off engine signals (`engine::functions-available`,
 * `engine::workers-available`, `trace`) rather than timers, so they are live
 * without polling.
 *
 * They ship as injected UI rather than console pages so the console SPA
 * keeps no per-view code: this bundle can be rebuilt, hot-reloaded, and
 * toggled off without touching the host.
 */

import type { Host } from '@iii-dev/console-ui'
import { FunctionsPage } from './src/catalog/FunctionsPage'
import { TriggersPage } from './src/catalog/TriggersPage'
import { WorkersPage } from './src/catalog/WorkersPage'

export default function setup(host: Host) {
  host.pages.register({
    id: 'functions',
    title: 'functions',
    render: () => <FunctionsPage host={host} />,
  })

  host.pages.register({
    id: 'triggers',
    title: 'triggers',
    render: () => <TriggersPage host={host} />,
  })

  // `fleet`, not `workers`: the console SPA still owns a native Workers tab,
  // and two nav entries with the same label would be a coin flip for the
  // operator. This page is the deeper read, and the native one can retire
  // once it is.
  host.pages.register({
    id: 'fleet',
    title: 'fleet',
    render: () => <WorkersPage host={host} />,
  })
}
