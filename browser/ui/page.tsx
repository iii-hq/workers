/**
 * Entry for the browser worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as browser/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers two contributions:
 * - src/page/ — the `#/ext/browser` page: the session rail, a screencast-fed
 *   live viewport, and the console/network feeds for the selected session.
 * - src/function-trigger-message/ — how every `browser::*` call renders in
 *   chat and the traces span tab (per-function terminal cards).
 *
 * No config form is registered: the browser worker's configuration is plain
 * scalar fields, so the console's schema-generated form is sufficient.
 *
 * Registrations go through `host` so the loader disposes them on hot reload /
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createBrowserRenderer } from './src/function-trigger-message'
import { BrowserPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'browser',
    title: 'browser',
    render: () => <BrowserPage host={host} />,
  })

  host.functionTriggers.register(createBrowserRenderer(host))
}
