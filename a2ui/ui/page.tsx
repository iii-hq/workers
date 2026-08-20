import type { Host } from '@iii-dev/console-ui'
import { A2uiPage } from './src/page'
import { createA2uiTriggerRenderer } from './src/trigger-renderer'

export default function setup(host: Host) {
  host.pages.register({
    id: 'a2ui',
    title: 'a2ui',
    render: (props) => <A2uiPage host={host} {...props} />,
  })
  host.functionTriggers.register(createA2uiTriggerRenderer(host))
}
