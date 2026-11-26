/**
 * Entry for the iii-cloud worker's injected console UI: esbuild bundles it
 * (react + @iii-dev/console-ui external) into dist/page.js, served as the
 * `iii-cloud/page.js` asset next to ../styles.css.
 *
 * `setup(host)` registers the `iii-cloud` page, the chat renderers of
 * connect/check/secret/deploy/watch, the configuration form, and one
 * palette command. See tech-specs/2026-11-iii-cloud-worker/spec.md §14.
 */

import type { Host } from '@iii-dev/console-ui'
import { CloudPage } from './src/CloudPage'
import { createConfigForm } from './src/ConfigForm'
import { createCloudRenderer } from './src/cards'
import { PAGE_ID } from './src/model'

export default function setup(host: Host) {
  host.pages.register({
    id: PAGE_ID,
    title: 'iii cloud',
    configurationId: 'iii-cloud',
    render: (props) => <CloudPage host={host} {...props} />,
  })
  host.functionCalls.register(createCloudRenderer(host))
  host.configForms.register('iii-cloud', createConfigForm(host))
  host.commands?.register(PAGE_ID, [
    {
      id: 'open',
      title: 'Open iii cloud',
      detail: 'Sign in, check readiness, and deploy to iii cloud',
      keywords: ['cloud', 'deploy', 'readiness', 'secrets'],
      run: () => host.panels?.open({ pageId: PAGE_ID, context: {} }),
    },
  ])
}
