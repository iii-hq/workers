/**
 * Entry for the database worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as database/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers three contributions:
 * - src/function-trigger-message/ — how every database::* call renders in
 *   chat and traces (SQL, request chips, result tables).
 * - src/page/ — the `#/ext/database` browser: schema tree, sortable row
 *   grid, row inspector, and a read-only SQL editor (shared Monaco). Reads
 *   the live database over `database::query`/`database::listDatabases`.
 * - src/configuration/ — the configuration form for the `database` entry
 *   on the Workers tab, replacing the generic schema-driven editor.
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { DatabaseConfigForm } from './src/configuration'
import { createDatabaseTriggerRenderer } from './src/function-trigger-message'
import { DatabasePage } from './src/page'

export default function setup(host: Host) {
  host.functionTriggers.register(createDatabaseTriggerRenderer(host))

  host.pages.register({
    id: 'database',
    title: 'database',
    render: () => <DatabasePage host={host} />,
  })

  host.configForms.register('database', (props) => (
    <DatabaseConfigForm {...props} host={host} />
  ))
}
