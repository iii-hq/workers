/**
 * Entry for the database worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * a `console::ui::asset` observer with `{ kind: 'script' }` (see src/ui.rs).
 * The stylesheet is its own asset: ../styles.css ships as a `style` asset,
 * database/styles.css — the console mounts and link-swaps it,
 * styles-before-scripts on boot.
 *
 * `setup(host)` registers three contributions:
 * - src/function-call-message/ — how every database::* call renders in
 *   chat and traces (SQL, request chips, result tables).
 * - src/page/ — the `database` page: schema tree, sortable row
 *   grid, row inspector, and a read-only SQL editor (shared Monaco). Reads
 *   the live database over `database::query`/`database::listDatabases`.
 *   Registered with the host's render props so the page follows the pane
 *   (side, per-tab state, the standard close affordance).
 * - src/configuration/ — the deliberate form for the `database` entry in
 *   global Settings.
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { DatabaseConfigForm } from './src/configuration'
import { createDatabaseCallRenderer } from './src/function-call-message'
import { DatabasePage } from './src/page'
import { registerDatabasePalette } from './src/palette'

export default function setup(host: Host) {
  host.functionCalls.register(createDatabaseCallRenderer(host))
  registerDatabasePalette(host)

  host.pages.register({
    id: 'database',
    title: 'database',
    configurationId: 'database',
    render: (props) => <DatabasePage host={host} {...props} />,
  })

  host.configForms.register('database', (props) => (
    <DatabaseConfigForm {...props} host={host} />
  ))
}
