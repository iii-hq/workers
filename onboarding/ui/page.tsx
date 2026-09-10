import type { Host } from '@iii-dev/console-ui'
import { TourPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'onboarding',
    title: 'Tour',
    render: (props) => <TourPage host={host} {...props} />,
  })
  host.commands?.register('onboarding', [
    {
      id: 'open',
      title: 'Open the iii tour',
      detail: 'Guided walkthrough of the console',
      keywords: ['onboarding', 'tour', 'walkthrough', 'help', 'tutorial'],
      run: () => host.panels?.open({ pageId: 'onboarding' }),
    },
  ])
}
