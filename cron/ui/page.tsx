import type { Host } from '@iii-dev/console-ui'
import { createCronObserverActivityRenderer } from './src/observer-activity'
import { CronSchedulesPage } from './src/page'
import { registerCronPalette } from './src/palette'

export default function setup(host: Host) {
  registerCronPalette(host)
  host.pages.register({
    id: 'cron',
    title: 'cron',
    render: (props) => <CronSchedulesPage host={host} {...props} />,
  })
  host.observerRenderers?.register(createCronObserverActivityRenderer())
}
