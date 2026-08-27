import type { Host, PageRenderProps } from '@iii-dev/console-ui'
import { ComposePage } from './src/page/index'

type Container = { container: string; state: string }
type Status = { namespace?: string; containers?: Container[] }

export default function setup(host: Host) {
  host.pages.register({
    id: 'compose',
    title: 'Compose',
    render: (props: PageRenderProps) => <ComposePage host={host} {...props} />,
  })

  host.commands?.register('compose', [
    {
      id: 'open',
      title: 'Open Compose',
      detail: 'Containers, lifecycle, worker packages, and logs for the compose project',
      keywords: ['workers', 'containers', 'services', 'project', 'supervisor', 'logs'],
      run: () => host.panels?.open({ pageId: 'compose', context: {} }),
    },
  ])

  host.palette?.registerSource({
    id: 'containers',
    title: 'Containers',
    kind: 'item',
    minQuery: 1,
    async search(query, { signal }) {
      const status = await host.iii.trigger<Status>('compose::status', {}, { timeoutMs: 10_000 })
      if (signal.aborted) return []
      const needle = query.trim().toLowerCase()
      return (status.containers ?? [])
        .filter((c) => c.container.toLowerCase().includes(needle))
        .slice(0, 30)
        .map((c) => ({
          id: c.container,
          title: c.container,
          detail: c.state,
          meta: status.namespace,
          run: () => host.panels?.open({ pageId: 'compose', context: { container: c.container } }),
        }))
    },
  })
}
