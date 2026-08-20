import { Button, EmptyState, IconButton, PageSidebar } from '@iii-dev/console-ui'
import { formatDate, StatusBadge, shortId } from '../components'
import type { EvalSummary } from '../types'

interface HistoryProps {
  side: 'left' | 'right'
  evaluations: EvalSummary[]
  selectedId: string | null
  loading: boolean
  onSelect: (evaluationId: string) => void
  onNew: () => void
  onRefresh: () => void
}

export function History({ side, evaluations, selectedId, loading, onSelect, onNew, onRefresh }: HistoryProps) {
  const tooltipSide = side === 'left' ? 'right' : 'left'

  return (
    <PageSidebar
      label="evaluation history"
      side={side}
      collapsible
      storageKey="eval:history"
      defaultWidth={280}
      narrowBelow={900}
      className="eval-ui-history"
      header={
        <div className="eval-ui-history-head">
          <span>recent evaluations</span>
          <Button variant="pill" size="sm" onClick={onRefresh} disabled={loading}>
            refresh
          </Button>
        </div>
      }
      collapsedActions={
        <>
          <IconButton label="new evaluation" tooltipSide={tooltipSide} onClick={onNew}>
            <PlusIcon />
          </IconButton>
          <IconButton
            label="refresh evaluation history"
            tooltipSide={tooltipSide}
            onClick={onRefresh}
            disabled={loading}
          >
            <RefreshIcon />
          </IconButton>
        </>
      }
    >
      <div className="eval-ui-history-content">
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
              const progress = evaluation.total_runs > 0 ? `${evaluation.terminal_runs}/${evaluation.total_runs}` : '—'
              return (
                <button
                  key={evaluation.evaluation_id}
                  type="button"
                  className={`eval-ui-history-row${selectedId === evaluation.evaluation_id ? ' active' : ''}`}
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
                    {formatDate(evaluation.created_at)} · {shortId(evaluation.evaluation_id)}
                  </span>
                </button>
              )
            })
          )}
        </div>
      </div>
    </PageSidebar>
  )
}

function PlusIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      aria-hidden
    >
      <title>new evaluation</title>
      <path d="M12 5v14M5 12h14" />
    </svg>
  )
}

function RefreshIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <title>refresh evaluation history</title>
      <path d="M20 11a8.1 8.1 0 0 0-15.5-2M4 4v5h5" />
      <path d="M4 13a8.1 8.1 0 0 0 15.5 2m.5 5v-5h-5" />
    </svg>
  )
}
