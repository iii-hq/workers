/**
 * Entry for the artifact worker's injected console UI, compiled into
 * dist/page.js and served as the `artifact/page.js` script asset (src/ui.rs).
 *
 * - src/page/                  — the Artifacts page: list, preview, Share…,
 *                                live links with Revoke
 * - src/function-call-message/ — chat cards for create/update/get/share with
 *                                a sandboxed preview, Share… and Open
 * - src/configuration/         — the form for the `artifact` configuration
 * - src/context-section/        — the Artifacts section of the chat's Context
 *                                panel: recent artifacts and live links
 */

import type { Host } from '@iii-dev/console-ui'
import { ArtifactConfigForm } from './src/configuration'
import { registerArtifactSection } from './src/context-section'
import { createArtifactCallRenderer } from './src/function-call-message'
import { PAGE_ID } from './src/lib/api'
import { ArtifactPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: PAGE_ID,
    title: 'Artifacts',
    configurationId: 'artifact',
    render: (props) => <ArtifactPage host={host} {...props} />,
  })
  host.functionCalls.register(createArtifactCallRenderer(host))
  host.configForms.register('artifact', ArtifactConfigForm)
  registerArtifactSection(host)
}
