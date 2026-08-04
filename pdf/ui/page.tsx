/**
 * Entry for the pdf worker's injected console UI — compiled by esbuild (react
 * and @iii-dev/console-ui external) into dist/page.js and served over the
 * `console:script` trigger (see src/ui.rs). The stylesheet is its own asset:
 * ./styles.css ships over `console:style` as pdf/styles.css.
 *
 * `setup(host)` composes two contributions:
 *
 * - src/page/                     — the PDF page (#/ext/pdf-reader)
 * - src/function-trigger-message/ — how pdf::* calls render in chat and traces
 *
 * Registrations go through `host` so the loader disposes them on hot reload and
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createPdfTriggerRenderer } from './src/function-trigger-message'
import { PdfPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'pdf-reader',
    title: 'pdf',
    render: () => <PdfPage host={host} />,
  })

  host.functionTriggers.register(createPdfTriggerRenderer(host))
}
