import type { Host } from '@iii-dev/console-ui'
import { ControlPlaneConfigForm } from './src/config-form'
import { ControlPlane } from './src/index'

export default async function setup(host: Host) {
  const { id: configurationId } = await host.iii.call<{ id: string }>('control-plane::configuration-id', {})
  const unregisterForm = host.configForms.register(configurationId, ControlPlaneConfigForm)
  const unregister = host.pages.register({
    id: 'control-plane',
    title: 'Control plane',
    configurationId,
    render: (props) => <ControlPlane host={host} configurationId={configurationId} {...props} />,
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
    unregisterForm()
  }
}
