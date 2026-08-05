/**
 * Entry for the console worker's engine-catalogue pages — compiled by
 * esbuild (react + @iii-dev/console-ui external) into dist/catalog-page.js
 * and served over the `console:script` trigger (see src/ui.rs). The
 * stylesheet is its own asset: styles.css ships over `console:style` as
 * console/styles.css.
 *
 * Two contributions, both reading engine-level data no single worker owns:
 *
 * - src/catalog/FunctionsPage — every registered function (#/ext/functions)
 * - src/catalog/TriggersPage  — trigger types and their live bindings
 *                               (#/ext/triggers)
 *
 * They ship as injected UI rather than console pages so the console SPA
 * keeps no per-view code: this bundle can be rebuilt, hot-reloaded, and
 * toggled off without touching the host.
 */

import type { Host } from '@iii-dev/console-ui'
import { FunctionsPage } from './src/catalog/FunctionsPage'
import { TriggersPage } from './src/catalog/TriggersPage'

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
}
