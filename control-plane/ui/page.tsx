import type { Host } from '@iii-dev/console-ui'
import { ControlPlane } from './src/index'

export default function setup(host: Host) {
  const unregister = host.pages.register({
    id: 'control-plane',
    title: 'Control plane',
    configurationId: 'control-plane',
    render: (props) => <ControlPlane host={host} {...props} />,
  })
  const unregisterCommands = host.commands?.register('control-plane', [
    {
      id: 'open',
      title: 'Open control plane',
      keywords: ['control plane', 'needs you', 'runs', 'decisions'],
      run: () => host.panels?.open({ pageId: 'control-plane' }),
    },
  ])
  return () => {
    unregisterCommands?.()
    unregister()
  }
}
