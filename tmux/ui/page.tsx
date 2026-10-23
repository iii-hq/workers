import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { TmuxConfigForm } from './src/configuration'
import { createOutputRouter } from './src/lib/output-router'
import { TmuxPage } from './src/page'

export default function setup(host: Host) {
  // One output handler for every terminal this script opens, owned here and
  // not by React, so Strict Mode's double effects never drop it.
  const router = createOutputRouter(host.iii)

  host.pages.register({
    id: 'tmux',
    title: 'tmux',
    configurationId: 'tmux',
    render: (props: PageRenderProps) => <TmuxPage host={host} router={router} {...props} />,
  })

  host.configForms.register('tmux', TmuxConfigForm)

  host.commands?.register('tmux', [
    {
      id: 'open',
      title: 'Open tmux',
      detail: 'Sessions, windows, agents at work and the ports behind them',
      keywords: ['terminal', 'session', 'window', 'claude', 'codex', 'compose', 'ports'],
      run: () => host.panels?.open({ pageId: 'tmux', context: {} }),
    },
  ])

  return () => router.dispose()
}
