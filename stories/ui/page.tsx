import type { Host } from '@iii-dev/console-ui'
import { StoriesConfigForm } from './src/config'
import { StoriesPage } from './src/explorer'
import { CONFIGURATION_ID, PAGE_ID } from './src/shared'

export default function setup(host: Host) {
  host.pages.register({
    id: PAGE_ID,
    title: 'Stories',
    configurationId: CONFIGURATION_ID,
    render: (props) => <StoriesPage {...props} host={host} />,
  })
  host.configForms.register(CONFIGURATION_ID, StoriesConfigForm)
}
