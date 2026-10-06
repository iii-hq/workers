/**
 * Entry for the ios-simulator worker's console UI (src/ui.rs serves the
 * built dist/page.js over `console:script`, styles.css over `console:style`).
 *
 * - src/page/ — the Simulators page: a device rail and a live iPhone you
 *   drive with multi-touch, buttons and the keyboard, plus its media.
 * - src/config.tsx — the purpose-built configuration form.
 * - src/renderer.tsx — screenshots in chat, like the browser worker's.
 * - src/overlay/ — the live preview: corner phones of the simulators an agent
 *   boots, opens or drives, like the browser worker's; expanding one opens
 *   the page.
 */

import type { Host } from '@iii-dev/console-ui'
import { IosSimulatorConfigForm } from './src/config'
import { SimulatorOverlay } from './src/overlay/SimulatorOverlay'
import { SimulatorsPage } from './src/page'
import { screenshotRenderer } from './src/renderer'

export default function setup(host: Host) {
  host.pages.register({
    id: 'ios-simulator',
    title: 'Simulators',
    configurationId: 'ios-simulator',
    render: (props) => <SimulatorsPage host={host} {...props} />,
  })
  host.configForms.register('ios-simulator', IosSimulatorConfigForm)
  host.functionTriggers.register(screenshotRenderer)
  // A console without the overlays slot shows no preview; the page still
  // reaches the simulator.
  host.overlays?.register({ id: 'ios-simulator-live', render: () => <SimulatorOverlay host={host} /> })
}
