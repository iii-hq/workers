import type { Host } from '@iii-dev/console-ui'
import { SentinelConfigForm } from './src/settings/SentinelConfigForm'
import { SentinelPage } from './src/page'
import { diagnosisRecordRenderer } from './src/renderers/diagnosis-record'
import { CONFIGURATION_ID, PAGE_ID } from './src/shared'

export default function setup(host: Host) {
  host.pages.register({
    id: PAGE_ID,
    title: 'errors',
    configurationId: CONFIGURATION_ID,
    render: (props) => <SentinelPage host={host} {...props} />,
  })
  host.configForms.register(CONFIGURATION_ID, (props) => (
    <SentinelConfigForm host={host} {...props} />
  ))
  // A diagnosis arrives in the transcript as a function call like any other.
  // Rendered, it reads as the finding it is instead of a JSON blob.
  host.functionTriggers.register(diagnosisRecordRenderer(host))
}
