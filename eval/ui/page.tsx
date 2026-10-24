import type { Host } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { createEvalApi } from './src/api'
import { badge, cycleTitle } from './src/model'
import { EvalPage } from './src/page'

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
      detail: 'Loop cycles',
      keywords: ['loop', 'cycle', 'evaluation'],
      run: () => host.panels?.open({ pageId: PAGE_ID, context: {} }),
    },
    {
      id: 'start-cycle',
      title: 'Start an eval cycle…',
      detail: 'From a finished E2E execution or an objective',
      keywords: ['objective', 'execution', 'evaluate', 'e2e'],
      run: () => host.panels?.open({ pageId: PAGE_ID, context: { type: 'start' } }),
    },
    {
      id: 'settings',
      title: 'Eval loop settings',
      detail: 'What the loop may do: measure, improve, prove and ship',
      keywords: ['config', 'budget', 'model', 'provider'],
      run: () => host.panels?.open({ pageId: PAGE_ID, context: { type: 'config' } }),
    },
  ])

  host.palette?.registerSource({
    id: 'eval-cycles',
    title: 'Eval cycles',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const cycles = await api.list().then(
        (list) => list.cycles,
        () => [],
      )
      if (signal.aborted) return []
      const needle = query.trim().toLowerCase()
      return cycles
        .filter((cycle) => [cycleTitle(cycle), cycle.cycle_id].some((field) => field.toLowerCase().includes(needle)))
        .slice(0, PALETTE_ROWS)
        .map((cycle) => ({
          id: cycle.cycle_id,
          title: cycleTitle(cycle),
          detail: `${badge(cycle).label} · ${cycle.cycle_id}`,
          meta: formatRelative(cycle.updated_at),
          keywords: [cycle.cycle_id],
          run: () => host.panels?.open({ pageId: PAGE_ID, context: { type: 'cycle', cycleId: cycle.cycle_id } }),
        }))
    },
  })
}
