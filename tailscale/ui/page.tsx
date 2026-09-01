import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { TailscalePage } from './src/page/index'

export default function setup(host: Host) {
  host.pages.register({
    id: 'tailscale',
    title: 'Tailscale',
    configurationId: 'tailscale',
    render: (props: PageRenderProps) => <TailscalePage host={host} {...props} />,
  })

  host.commands?.register('tailscale', [
    {
      id: 'open',
      title: 'Open Tailscale',
      detail: 'Share the Console over your tailnet with a link and QR code',
      keywords: ['remote', 'mobile', 'phone', 'serve', 'funnel', 'qr'],
      run: () => host.panels?.open({ pageId: 'tailscale', context: {} }),
    },
  ])
}
