/**
 * Entry for the memory worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ../styles.css ships over `console:style` as memory/styles.css —
 * the console mounts and link-swaps it, styles-before-scripts on boot.
 *
 * `setup(host)` registers one contribution:
 * - src/page/ — the `#/ext/memory` page: banks in a navigation rail, the
 *   selected bank's workspace (always-injected markdown rules, memories with
 *   pin/edit/tombstone in place, a schematic graph, and a turn-preview dry
 *   run) behind one segmented control; a drill-in flow when the pane is
 *   narrow. Everything re-reads live off `memory::item-changed` /
 *   `memory::bank-changed` (poll fallback while the bindings are
 *   unavailable).
 *
 * The page declares its configuration id so the Console adds the standard
 * settings action. Its purpose-built form ships with the Console's shared
 * configuration-form asset. Memory ships no custom chat cards.
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { MemoryPage } from './src/page'
import { registerMemoryPalette } from './src/page/palette'

export default function setup(host: Host) {
  host.pages.register({
    id: 'memory',
    title: 'memory',
    configurationId: 'memory',
    render: (props) => <MemoryPage host={host} {...props} />,
  })
  registerMemoryPalette(host)
}
