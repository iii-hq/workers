import type { Host } from '@iii-dev/console-ui'
import { StorageConfigForm } from './src/configuration'
import { StoragePage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'storage-manager',
    title: 'storage',
    render: (props) => <StoragePage host={host} {...props} />,
  })
  host.configForms.register('storage', StorageConfigForm, { layout: 'full' })
}
