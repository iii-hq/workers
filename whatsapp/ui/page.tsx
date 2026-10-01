import type { Host } from '@iii-dev/console-ui'
import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { WhatsAppConfigForm } from './src/configuration'
import { WhatsAppPage } from './src/page'
import { loadDisplayFont } from './src/font'

export default async function setup(host: Host) {
  loadDisplayFont()
  // Resolve through this addressed worker, including a project namespace. Never choose another entry.
  const configurationId = await resolveConfigurationId(host.iii, 'whatsapp')
  host.configForms.register('whatsapp', (props) => <WhatsAppConfigForm host={host} {...props} />)
  host.pages.register({
    id: 'whatsapp-manager', title: 'WhatsApp', configurationId,
    render: (props) => <WhatsAppPage host={host} configurationId={configurationId} {...props} />,
  })
  host.commands?.register('whatsapp-manager', [{
    id: 'open', title: 'Open WhatsApp', keywords: ['pair', 'session', 'message', 'group'],
    run: () => host.panels?.open({ pageId: 'whatsapp-manager' }),
  }])
}
