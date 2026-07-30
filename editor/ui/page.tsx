/**
 * Entry for the editor worker's injected console UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/page.js and served over the
 * `console:script` trigger (see src/ui.rs). The stylesheet is its own asset:
 * styles.css ships over `console:style` as editor/styles.css.
 *
 * `setup(host)` registers two contributions:
 * - src/function-trigger-message/ — how every editor::* call renders in chat
 *   and traces (diffs as diffs, saves as file cards).
 * - src/page/ — the `#/ext/editor` page: changed files, tabs, the shared
 *   Monaco editor, unsaved-diff view, and the live feed of edits landing.
 *
 * Registrations go through `host` so the loader disposes them on hot reload
 * and on worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { createEditorTriggerRenderer } from './src/function-trigger-message'
import { EditorPage } from './src/page'

export default function setup(host: Host) {
  host.functionTriggers.register(createEditorTriggerRenderer(host))

  host.pages.register({
    id: 'editor',
    title: 'editor',
    render: () => <EditorPage host={host} />,
  })
}
