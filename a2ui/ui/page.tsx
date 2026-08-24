import type { Host } from '@iii-dev/console-ui'
import { listSurfaces } from './src/data'
import { A2uiPage } from './src/page'
import { createA2uiTriggerRenderer } from './src/trigger-renderer'

export default function setup(host: Host) {
  host.pages.register({
    id: 'a2ui',
    title: 'A2UI',
    render: (props) => <A2uiPage host={host} {...props} />,
  })
  host.functionTriggers.register(createA2uiTriggerRenderer(host))

  host.palette?.registerSource({
    id: 'surfaces',
    title: 'A2UI surfaces',
    kind: 'item',
    minQuery: 2,
    async search(query, { conversationId, signal }) {
      if (!conversationId) return []
      const { surfaces } = await listSurfaces(host, conversationId)
      if (signal.aborted) return []
      const needle = query.toLowerCase()
      return surfaces
        .filter(
          (surface) =>
            surface.title.toLowerCase().includes(needle) ||
            surface.surface_id.toLowerCase().includes(needle),
        )
        .slice(0, 30)
        .map((surface) => ({
          id: surface.surface_id,
          title: surface.title || surface.surface_id,
          detail: `${surface.component_count} components`,
          keywords: [surface.surface_id],
          run: () =>
            host.panels?.open({
              pageId: 'a2ui',
              context: { surfaceId: surface.surface_id },
            }),
        }))
    },
  })

  host.commands?.register('a2ui', [
    {
      id: 'open',
      title: 'Open A2UI',
      detail: 'Generative interfaces for this conversation',
      keywords: ['surfaces', 'generative ui', 'interface'],
      run: () => host.panels?.open({ pageId: 'a2ui', context: {} }),
    },
  ])
}
