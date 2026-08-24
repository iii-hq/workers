/**
 * Entry for the iii-directory worker's injected console UI — compiled by
 * esbuild (react + @iii-dev/console-ui external) into dist/page.js and
 * served over the `console:script` trigger (see src/ui.rs). The stylesheet
 * is its own asset: ../styles.css ships over `console:style` as
 * iii-directory/styles.css.
 *
 * `setup(host)` composes the worker's four console contributions, one
 * module each:
 *
 * - src/page/             — the skills & prompts browser/editor (#/ext/directory)
 * - src/configuration/    — custom form for the `iii-directory` configuration entry
 * - src/function-trigger/ — how directory::* function triggers render in chat/traces
 * - src/session-chip/     — the chat header's system-prompt read-out + dialog
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { DirectoryConfigForm } from './src/configuration'
import { createDirectoryTriggerRenderer } from './src/function-trigger'
import { DirectoryPage } from './src/page'
import { registerDirectoryPalette } from './src/page/palette'
import { createSearchTriggerRenderer } from './src/search/search-card'
import { createSystemPromptChip } from './src/session-chip'

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
    render: (props) => <DirectoryPage host={host} {...props} />,
  })

  registerDirectoryPalette(host)

  host.functionTriggers.register(createDirectoryTriggerRenderer())
  host.functionTriggers.register(createSearchTriggerRenderer())

  host.configForms.register('iii-directory', DirectoryConfigForm)

  host.chat?.registerTranscriptRenderer?.({
    id: 'directory',
    render: DirectoryPassLine,
  })

  /* Optional chained: the slot postdates the published Host type, so a
     console that predates session chips just skips this contribution. */
  host.chat?.registerSessionChip({
    id: 'system-prompt',
    render: createSystemPromptChip(host),
  })
}
