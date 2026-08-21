import type { Host } from '@iii-dev/console-ui'
import { CronConfigForm } from './src/configuration'
import { CronSchedulesPage } from './src/page'
import { createCronTriggerActivityRenderer } from './src/trigger-activity'

export default function setup(host: Host) {
  host.pages.register({
    id: 'cron',
    title: 'cron',
    render: (props) => <CronSchedulesPage host={host} {...props} />,
  })
  host.configForms.register('cron', CronConfigForm)
  host.triggerRenderers?.register(createCronTriggerActivityRenderer())
}
