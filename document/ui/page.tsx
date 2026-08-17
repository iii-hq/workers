/**
 * Entry for the document worker's injected console UI — compiled by esbuild
 * (react and @iii-dev/console-ui external) into dist/page.js and served over
 * the `console:script` trigger (see src/ui.rs). The stylesheet is its own
 * asset: ./styles.css ships over `console:style` as document/styles.css.
 *
 * `setup(host)` composes two contributions:
 *
 * - src/page/                     — the document page (#/ext/document-reader)
 * - src/function-trigger-message/ — how document::* calls render in chat and
 *                                   traces
 *
 * Registrations go through `host` so the loader disposes them on hot reload and
 * worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createDocumentTriggerRenderer } from './src/function-trigger-message'
import { DocumentPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'document-reader',
    title: 'documents',
    render: () => <DocumentPage host={host} />,
  })

  host.functionTriggers.register(createDocumentTriggerRenderer(host))
}
