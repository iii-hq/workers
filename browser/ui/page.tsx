/**
 * Entry for the browser worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as browser/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers four contributions:
 * - src/page/ — the `browser` page: a browser — a Chrome-style tab
 *   strip, the address bar, a screencast-fed live viewport, and the
 *   developer-tools dock (console/network/downloads/history) behind the menu.
 * - src/function-trigger-message/ — how every `browser::*` call renders in
 *   chat and the traces span tab (per-function terminal cards).
 * - src/configuration/ — the full-width, purpose-built browser settings
 *   editor used inside the Console's global Settings modal.
 * - src/overlay/ — the live preview: a corner thumbnail of the tab an agent
 *   just opened, with expand (opens the page) and hide.
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
import { BrowserOverlay } from './src/overlay/BrowserOverlay'
import { BrowserPage } from './src/page'
import { registerBrowserPalette } from './src/page/palette'

export default function setup(host: Host) {
  host.pages.register({
    id: 'browser',
    title: 'browser',
    configurationId: 'browser',
    render: (props) => <BrowserPage host={host} {...props} />,
  })

  registerBrowserPalette(host)

  // The form needs the bus for its "Clear browser data" action.
  host.configForms.register(
    'browser',
    (props) => <BrowserConfigForm {...props} iii={host.iii} />,
    {
      layout: 'full',
    },
  )

  // A captured page is a first-class chat artifact. Register its focused
  // renderer first; the general browser renderer still owns errors/running
  // states and every other browser::* function.
  host.functionTriggers.register(createBrowserScreenshotRenderer())
  host.functionTriggers.register(createScraplingRenderer(host))
  host.functionTriggers.register(createBrowserRenderer(host))

  // A session an agent starts shows up as a small live preview in a corner
  // (the `overlays` slot) instead of pulling the browser page into the
  // workspace, which flipped the user's tab under them on every test run.
  // Expanding the preview is the one thing that opens the page; the page's
  // own new tabs and an explicit "Open in browser" never get one. A console
  // without the slot shows nothing automatic — the chat card's link still
  // opens the page.
  host.overlays?.register({
    id: 'browser-live',
    render: () => <BrowserOverlay host={host} />,
  })
}
