import type { Host } from '@iii-dev/console-ui'
import { Tower } from './src/index'

export default function setup(host: Host) {
  const unregister = host.pages.register({
    id: 'tower',
    title: 'Control tower',
    configurationId: 'tower',
    render: (props) => <Tower host={host} {...props} />,
  })
  const unregisterCommands = host.commands?.register('tower', [
    {
      id: 'open',
      title: 'Open control tower',
      keywords: ['tower', 'needs you', 'runs', 'decisions'],
      run: () => host.panels?.open({ pageId: 'tower' }),
    },
  ])
  return () => {
    unregisterCommands?.()
    unregister()
  }
}
