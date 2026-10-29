/**
 * Entry for the pdf worker's injected console UI — compiled by esbuild (react
 * and @iii-dev/console-ui external) into dist/page.js and served over a
 * `console::ui::asset` observer with `{ kind: 'script' }` (see src/ui.rs). The
 * stylesheet is its own asset: ./styles.css ships as a `style` asset,
 * pdf/styles.css.
 *
 * `setup(host)` composes two contributions:
 *
 * - src/page/                     — the PDF page (page `pdf-reader`)
 * - src/function-call-message/ — how pdf::* calls render in chat and traces
 *
 * Registrations go through `host` so the loader disposes them on hot reload and
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createPdfCallRenderer } from './src/function-call-message'
import { PdfPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'pdf-reader',
    title: 'pdf',
    configurationId: 'pdf',
    render: (props) => <PdfPage host={host} {...props} />,
  })

  host.functionCalls.register(createPdfCallRenderer(host))

  host.commands?.register('pdf-reader', [
    {
      id: 'open',
      title: 'Open pdf',
      detail: 'Classify a document and read its extracted markdown',
      keywords: ['document', 'ocr', 'markdown'],
      run: () => host.panels?.open({ pageId: 'pdf-reader', context: {} }),
    },
  ])
}
