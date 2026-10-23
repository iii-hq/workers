import type { Host } from '@iii-dev/console-ui'
import { createEvalApi } from './src/api'
import { statusPresentation } from './src/model'
import { EvalPage } from './src/page'
import { clock, rowTitle } from './src/page/monitor/shell-state'

const PAGE_ID = 'eval-benchmarks'
const PALETTE_ROWS = 30

export default function setup(host: Host) {
  const api = createEvalApi(host)

  host.pages.register({
    id: PAGE_ID,
    title: 'eval',
    render: (props) => <EvalPage host={host} {...props} />,
  })

  host.commands?.register(PAGE_ID, [
    {
      id: 'open',
      title: 'Open eval',
      detail: 'Session monitor',
      keywords: ['monitor', 'analysis', 'sessions'],
      run: () => host.panels?.open({ pageId: PAGE_ID, context: {} }),
    },
    {
      id: 'analyze-session',
      title: 'Analyze a session…',
      detail: 'Run the session monitor on a finished session',
      keywords: ['analyze', 'monitor', 'session', 'suggestions'],
      run: () => host.panels?.open({ pageId: PAGE_ID, context: { type: 'analyze' } }),
    },
  ])

  host.palette?.registerSource({
    id: 'eval-analyses',
    title: 'Session monitor',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const records = await api.list().catch(() => [])
      if (signal.aborted) return []
      const needle = query.trim().toLowerCase()
      return records
        .filter((record) =>
          [record.source_title, record.session_id, record.evaluation_id].some((field) =>
            field?.toLowerCase().includes(needle),
          ),
        )
        .slice(0, PALETTE_ROWS)
        .map((record) => ({
          id: record.evaluation_id,
          title: rowTitle(record),
          detail: `${statusPresentation(record).label} · ${record.session_id}`,
          meta: clock(record.created_at),
          keywords: [record.session_id, record.evaluation_id],
          run: () =>
            host.panels?.open({
              pageId: PAGE_ID,
              context: { type: 'analysis', evaluationId: record.evaluation_id },
            }),
        }))
    },
  })
}
