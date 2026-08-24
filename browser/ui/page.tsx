/**
 * Entry for the browser worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as browser/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers three contributions:
 * - src/page/ — the `#/ext/browser` page: the session rail, a screencast-fed
 *   live viewport, and the console/network feeds for the selected session;
 *   drill-in flow (list ⇄ session workspace) when the pane is narrow.
 * - src/function-trigger-message/ — how every `browser::*` call renders in
 *   chat and the traces span tab (per-function terminal cards).
 * - src/configuration/ — the full-width, purpose-built browser settings
 *   editor used by the Console's worker configuration dialog.
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { BrowserConfigForm } from './src/configuration'
import {
  createBrowserRenderer,
  createBrowserScreenshotRenderer,
} from './src/function-trigger-message'
import { createScraplingRenderer } from './src/function-trigger-message/scrapling'
import { BrowserPage } from './src/page'
import { registerBrowserPalette } from './src/page/palette'

export default function setup(host: Host) {
  host.pages.register({
    id: 'browser',
    title: 'browser',
    render: (props) => <BrowserPage host={host} {...props} />,
  })

  registerBrowserPalette(host)

  host.configForms.register('browser', BrowserConfigForm, { layout: 'full' })

  // A captured page is a first-class chat artifact. Register its focused
  // renderer first; the general browser renderer still owns errors/running
  // states and every other browser::* function.
  host.functionTriggers.register(createBrowserScreenshotRenderer())
  host.functionTriggers.register(createScraplingRenderer(host))
  host.functionTriggers.register(createBrowserRenderer(host))
}
