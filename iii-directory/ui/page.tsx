/**
 * Entry for the iii-directory worker's injected console UI — compiled by
 * esbuild (react + @iii-dev/console-ui external) into dist/page.js and
 * served over the `console:script` trigger (see src/ui.rs). The stylesheet
 * is its own asset: ../styles.css ships over `console:style` as
 * iii-directory/styles.css.
 *
 * `setup(host)` composes the worker's three console contributions, one
 * module each:
 *
 * - src/page/             — the skills & prompts browser/editor (#/ext/directory)
 * - src/configuration/    — custom form for the `iii-directory` configuration entry
 * - src/function-trigger/ — how directory::* function triggers render in chat/traces
 *
 * Registrations go through `host` so the loader disposes them on hot
 * reload / worker disconnect.
 */

import type { Host } from '@iii-dev/console-ui'
import { DirectoryConfigForm } from './src/configuration'
import { createDirectoryTriggerRenderer } from './src/function-trigger'
import { DirectoryPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'directory',
    title: 'directory',
    render: (props) => <DirectoryPage host={host} {...props} />,
  })

  host.functionTriggers.register(createDirectoryTriggerRenderer())

  host.configForms.register('iii-directory', DirectoryConfigForm)
}
