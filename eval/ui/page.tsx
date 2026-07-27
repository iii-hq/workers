import type { Host } from '@iii-dev/console-ui'
import { EvalPage } from './src/page'

export default function setup(host: Host) {
  host.pages.register({
    id: 'eval-benchmarks',
    title: 'eval',
    render: () => <EvalPage host={host} />,
  })
}
