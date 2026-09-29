import type { Host } from '@iii-dev/console-ui'
import { OnboardingPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'onboarding',
    title: 'onboarding',
    render: (props) => <OnboardingPage host={host} {...props} />,
  })
  host.commands?.register('onboarding', [
    {
      id: 'open',
      title: 'Open onboarding',
      detail: 'Guided walkthrough of the console',
      keywords: ['onboarding', 'tour', 'walkthrough', 'help', 'tutorial'],
      run: () => host.panels?.open({ pageId: 'onboarding' }),
    },
  ])
}
