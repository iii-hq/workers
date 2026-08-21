import type { Host } from '@iii-dev/console-ui'
import { createEvalApi } from './src/api'
import { EvalPage } from './src/page'

const PALETTE_ROWS = 30

export default function setup(host: Host) {
  const api = createEvalApi(host)

  host.pages.register({
    id: 'eval-benchmarks',
    title: 'eval',
    render: (props) => <EvalPage host={host} {...props} />,
  })

  host.commands?.register('eval-benchmarks', [
    {
      id: 'open',
      title: 'Open eval',
      detail: 'Compare sessions and prompt experiments',
      keywords: ['evaluation', 'benchmark', 'compare'],
      run: () => host.panels?.open({ pageId: 'eval-benchmarks', context: {} }),
    },
    {
      id: 'new-evaluation',
      title: 'New evaluation…',
      detail: 'Start a prompt comparison',
      keywords: ['evaluation', 'experiment', 'compare prompts'],
      run: () =>
        host.panels?.open({
          pageId: 'eval-benchmarks',
          context: { type: 'new' },
        }),
    },
  ])

  host.palette?.registerSource({
    id: 'evaluations',
    title: 'Evaluations',
    kind: 'item',
    minQuery: 2,
    async search(query, { signal }) {
      const evaluations = await api.list()
      if (signal.aborted) return []
      const needle = query.trim().toLowerCase()
      return evaluations
        .filter((evaluation) =>
          `${evaluation.control_label ?? ''} ${evaluation.treatment_label ?? ''} ${evaluation.model} ${evaluation.dimension}`
            .toLowerCase()
            .includes(needle),
        )
        .slice(0, PALETTE_ROWS)
        .map((evaluation) => ({
          id: evaluation.evaluation_id,
          title: `${evaluation.control_label ?? 'control'} vs ${evaluation.treatment_label ?? 'treatment'}`,
          detail: `${evaluation.model} · ${evaluation.dimension}`,
          run: () =>
            host.panels?.open({
              pageId: 'eval-benchmarks',
              context: {
                type: 'evaluation',
                evaluationId: evaluation.evaluation_id,
              },
            }),
        }))
    },
  })
}
