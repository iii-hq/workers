import type { Host } from '@iii-dev/console-ui'
import { SlidesPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'slides',
    title: 'Slides',
    configurationId: 'slides',
    render: (props) => <SlidesPage host={host} {...props} />,
  })

  host.commands?.register('slides', [
    {
      id: 'open',
      title: 'Open Slides',
      detail: 'Edit, present and export slide decks',
      keywords: ['deck', 'presentation', 'pptx', 'pdf'],
      run: () => host.panels?.open({ pageId: 'slides', context: {} }),
    },
  ])
}
