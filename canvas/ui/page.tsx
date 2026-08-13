/**
 * Entry for the canvas worker's injected console UI — compiled by esbuild
 * (react and @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ./styles.css ships over `console:style` as canvas/styles.css. The
 * mermaid and freeform vendor bundles are separate script assets, lazily
 * imported through src/lib/loaders.ts.
 *
 * `setup(host)` composes two contributions:
 *
 * - src/page/                     — the canvas page (#/ext/canvas)
 * - src/function-trigger-message/ — how canvas::* calls render in chat/traces
 *
 * Registrations go through `host` so the loader disposes them on hot reload
 * and worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createCanvasTriggerRenderer } from './src/function-trigger-message'
import { CanvasPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'canvas',
    title: 'canvas',
    render: (props) => <CanvasPage host={host} {...props} />,
  })

  host.functionTriggers.register(createCanvasTriggerRenderer(host))
}
