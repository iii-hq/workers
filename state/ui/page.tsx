/**
 * Entry for the state worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over the
 * `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as state/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` composes the worker's three console contributions, one
 * module each:
 *
 * - src/page/                    — the state-manager page (#/ext/state-manager):
 *                                  scopes × keys columns over a Monaco JSON
 *                                  value editor, drill-in flow when narrow
 * - src/configuration/           — custom form for the `state` configuration entry
 * - src/function-trigger-message/ — how state::set / state::get triggers render
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { StateConfigForm } from './src/configuration'
import { createStateTriggerRenderer } from './src/function-trigger-message'
import { StateManagerPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'state-manager',
    title: 'state',
    render: (props) => <StateManagerPage host={host} {...props} />,
  })

  host.functionTriggers.register(createStateTriggerRenderer(host))

  host.configForms.register('state', StateConfigForm)
}
