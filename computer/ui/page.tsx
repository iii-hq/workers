/**
 * Entry for the computer worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over the
 * `console:script` trigger (see src/ui.rs). The stylesheet is its own asset:
 * styles.css ships over `console:style` as computer/styles.css — the console
 * mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers two contributions:
 * - src/page/ — the `#/ext/computer` page: session rail plus a screencast-fed
 *   live desktop that forwards clicks, scroll and typing as `computer::act`;
 *   drill-in flow (list ⇄ viewport) when the pane is narrow.
 * - src/function-trigger-message/ — how every `computer::*` call renders in
 *   chat and the traces span tab.
 *
 * The page declares its configuration id so the Console adds the standard
 * settings action. Its purpose-built form ships with the Console's shared
 * configuration-form asset.
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createComputerRenderer } from './src/function-trigger-message'
import { ComputerPage } from './src/page'
import { registerComputerPalette } from './src/page/palette'

export default function setup(host: Host) {
  host.pages.register({
    id: 'computer',
    title: 'computer',
    configurationId: 'computer',
    render: (props) => <ComputerPage host={host} {...props} />,
  })

  registerComputerPalette(host)

  host.functionTriggers.register(createComputerRenderer(host))
}
