import type { Host } from '@iii-dev/console-ui'
import { SecurityScanPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'security-scan',
    title: 'security scans',
    configurationId: 'security-scan',
    render: (props) => <SecurityScanPage host={host} {...props} />,
  })
}
