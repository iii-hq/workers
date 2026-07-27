import { Button, EmptyState } from '@iii-dev/console-ui'
import { formatDate, shortId, StatusBadge } from '../components'
import type { EvalSummary } from '../types'

export function History({
  evaluations,
  selectedId,
  loading,
  onSelect,
  onNew,
  onRefresh,
}: {
  evaluations: EvalSummary[]
  selectedId: string | null
  loading: boolean
  onSelect: (evaluationId: string) => void
  onNew: () => void
  onRefresh: () => void
}) {
  return (
    <aside className="eval-ui-history">
      <div className="eval-ui-history-head">
        <span>recent evaluations</span>
        <Button variant="pill" size="sm" onClick={onRefresh} disabled={loading}>
          refresh
        </Button>
      </div>
      <Button variant="primary" size="sm" onClick={onNew}>
        new evaluation
      </Button>
      <div className="eval-ui-history-list">
        {evaluations.length === 0 && !loading ? (
          <EmptyState
            title="no evaluations yet"
            description="compare a baseline and candidate prompt to create the first report."
            action={{ label: 'new evaluation', onClick: onNew }}
          />
        ) : (
          evaluations.map((evaluation) => {
            const control = evaluation.control_label ?? 'control'
            const treatment = evaluation.treatment_label ?? 'treatment'
            const progress =
              evaluation.total_runs > 0
                ? `${evaluation.terminal_runs}/${evaluation.total_runs}`
                : '—'
            return (
              <button
                key={evaluation.evaluation_id}
                type="button"
                className={`eval-ui-history-row${
                  selectedId === evaluation.evaluation_id ? ' active' : ''
                }`}
                onClick={() => onSelect(evaluation.evaluation_id)}
              >
                <span className="eval-ui-history-row-top">
                  <StatusBadge status={evaluation.status} />
                  <span className="eval-ui-history-progress">{progress}</span>
                </span>
                <span className="eval-ui-history-title">
                  {control} vs {treatment}
                </span>
                <span className="eval-ui-history-meta">
                  {evaluation.model} · {evaluation.dimension.replace('_', ' ')}
                </span>
                <span className="eval-ui-history-meta">
                  {formatDate(evaluation.created_at)} ·{' '}
                  {shortId(evaluation.evaluation_id)}
                </span>
              </button>
            )
          })
        )}
      </div>
    </aside>
  )
}
