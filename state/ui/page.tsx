/**
 * Entry for the state worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served as a
 * `script` asset of `console::ui::asset` (see src/ui.rs). The stylesheet is
 * its own asset: ../styles.css ships as the `style` asset state/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` composes the worker's three console contributions, one
 * module each:
 *
 * - src/page/                    — the state-manager page (page `state-manager`):
 *                                  scopes × keys columns over a Monaco JSON
 *                                  value editor, drill-in flow when narrow
 * - src/configuration/           — custom form for the `state` configuration entry
 * - src/function-call-message/    — how state::set / state::get calls render
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { StateConfigForm } from './src/configuration'
import { createStateCallRenderer } from './src/function-call-message'
import { StateManagerPage } from './src/page'
import { registerStatePalette } from './src/palette'

export default function setup(host: Host) {
  registerStatePalette(host)
  host.pages.register({
    id: 'state-manager',
    title: 'state',
    configurationId: 'state',
    render: (props) => <StateManagerPage host={host} {...props} />,
  })

  host.functionCalls.register(createStateCallRenderer(host))

  host.configForms.register('state', StateConfigForm)
}
