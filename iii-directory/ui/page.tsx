/**
 * Entry for the iii-directory worker's injected console UI — compiled by
 * esbuild (react + @iii-dev/console-ui external) into dist/page.js and
 * served as a `script` asset of `console::ui::asset` (see src/ui.rs). The
 * stylesheet is its own `style` asset: ../styles.css ships as
 * iii-directory/styles.css.
 *
 * `setup(host)` composes the worker's four console contributions, one
 * module each:
 *
 * - src/page/             — the skills & agent profiles browser/editor
 * - src/configuration/    — custom form for the `iii-directory` configuration entry
 * - src/function-call/    — how directory::* function calls render in chat/traces
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { DirectoryConfigForm } from './src/configuration'
import { createDirectoryCallRenderer } from './src/function-call'
import { DirectoryPage } from './src/page'
import { registerDirectoryPalette } from './src/page/palette'
import { createSearchCallRenderer } from './src/search/search-card'

/**
 * The pre-generate hook's transcript annotations (`origin.directory`) stay
 * in the durable data for traces and measurement but render nothing in
 * chat: the null renderer SUPPRESSES the console's fallback summary row —
 * without it every generation would print "directory · hint injected"/
 * "directory · skipped" lines.
 */
function DirectoryPassLine() {
  return null
}

export default function setup(host: Host) {
  host.pages.register({
    id: 'directory',
    title: 'Directory',
    configurationId: 'iii-directory',
    render: (props) => <DirectoryPage host={host} {...props} />,
  })

  registerDirectoryPalette(host)

  host.functionCalls.register(createDirectoryCallRenderer(host))
  host.functionCalls.register(createSearchCallRenderer())

  host.configForms.register('iii-directory', DirectoryConfigForm)

  host.chat?.registerTranscriptRenderer?.({
    id: 'directory',
    render: DirectoryPassLine,
  })

}
