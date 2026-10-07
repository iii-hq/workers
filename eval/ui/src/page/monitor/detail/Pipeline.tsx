// The five steps of an analysis with what each took. Wide: five tiles.
// Narrow: a five-segment bar and one mono line of times.
import { uiClasses } from '@iii-dev/console-ui'
import { Ban, Check, Circle, Clock, LoaderCircle, Minus, X } from 'lucide-react'
import { isActive, type PipelineStep, pipeline, type StepState } from '../../../model'
import type { AnalysisRecord } from '../../../types'
import { plural, seconds } from './present'

const STATE_WORD: Record<StepState, string> = {
  done: 'done',
  running: 'running',
  waiting: 'waiting',
  failed: 'failed',
  cancelled: 'cancelled',
  skipped: 'not needed',
  pending: 'not reached',
}

const SHORT: Record<PipelineStep['key'], string> = {
  queued: 'Queued',
  collecting: 'Collect',
  judging: 'Triage',
  investigating: 'Investigate',
  done: 'Done',
}

function StepIcon({ state }: { state: StepState }) {
  switch (state) {
    case 'done':
      return <Check size={16} aria-hidden="true" />
    case 'running':
      return <LoaderCircle size={16} aria-hidden="true" className={uiClasses.spin} />
    case 'waiting':
      return <Clock size={16} aria-hidden="true" />
    case 'failed':
      return <X size={16} aria-hidden="true" />
    case 'cancelled':
      return <Ban size={16} aria-hidden="true" />
    case 'skipped':
      return <Minus size={16} aria-hidden="true" />
    case 'pending':
      return <Circle size={16} aria-hidden="true" />
  }
}

interface Timing {
  steps: PipelineStep[]
  /** Time the analysis has taken, or took. */
  elapsed: number
  budget: number
  terminal: boolean
}

function timing(record: AnalysisRecord, now: number): Timing {
  const terminal = !isActive(record.status)
  const end = record.completed_at ?? (terminal ? record.updated_at : now)
  return {
    steps: pipeline(record, now),
    elapsed: Math.max(0, end - record.created_at),
    budget: Math.max(0, record.deadline - record.created_at),
    terminal,
  }
}

/** The quiet second line of a tile: `2.1 s · 3 sessions`, `Not needed`. */
function tileMeta(step: PipelineStep, record: AnalysisRecord, t: Timing): string {
  if (step.state === 'skipped') return 'Not needed'
  if (step.key === 'done') return `${seconds(t.elapsed)} of ${seconds(t.budget, 0)}`
  if (step.durationMs === undefined) return t.terminal ? 'Not reached' : '—'
  const extra =
    step.key === 'collecting' && record.counters.sessions > 0
      ? plural(record.counters.sessions, 'session')
      : step.key === 'judging'
        ? 'Jev'
        : step.key === 'investigating'
          ? 'LLM'
          : undefined
  return [seconds(step.durationMs), extra].filter(Boolean).join(' · ')
}

/** `Queued 0.2 s · Collect 2.1 s · … · Done at 47.3 s of 180 s` */
function timesLine(record: AnalysisRecord, t: Timing): string {
  const parts = t.steps.map((step) => {
    if (step.key === 'done') {
      const at = `${seconds(t.elapsed)} of ${seconds(t.budget, 0)}`
      if (record.status === 'completed') return `Done at ${at}`
      if (record.status === 'failed') return `Failed at ${at}`
      if (record.status === 'cancelled') return `Cancelled at ${at}`
      return at
    }
    if (step.state === 'skipped') return `${SHORT[step.key]} not needed`
    return step.durationMs === undefined ? undefined : `${SHORT[step.key]} ${seconds(step.durationMs)}`
  })
  return parts.filter(Boolean).join(' · ')
}

export function Pipeline({ record, now, narrow }: { record: AnalysisRecord; now: number; narrow: boolean }) {
  const t = timing(record, now)

  if (narrow) {
    return (
      <section aria-label="Analysis steps" className="eval-ui-ad-compact">
        <ol className="eval-ui-ad-bar">
          {t.steps.map((step) => (
            <li key={step.key} data-phase={step.state}>
              <span className="eval-ui-ad-sr">
                {step.label}: {STATE_WORD[step.state]}
              </span>
            </li>
          ))}
        </ol>
        <ol className="eval-ui-ad-bar-labels" aria-hidden="true">
          {t.steps.map((step) => (
            <li key={step.key}>
              {step.key === 'done'
                ? record.status === 'failed'
                  ? 'Failed'
                  : record.status === 'cancelled'
                    ? 'Cancelled'
                    : 'Done'
                : SHORT[step.key]}
            </li>
          ))}
        </ol>
        <p className="eval-ui-ad-times">{timesLine(record, t)}</p>
      </section>
    )
  }

  return (
    <ol aria-label="Analysis steps" className="eval-ui-ad-steps">
      {t.steps.map((step) => (
        <li
          key={step.key}
          className="eval-ui-ad-step"
          data-phase={step.state}
          data-final={step.key === 'done' || undefined}
        >
          <span className="eval-ui-ad-step-label">
            <StepIcon state={step.state} />
            {step.label}
            <span className="eval-ui-ad-sr">: {STATE_WORD[step.state]}</span>
          </span>
          <span className="eval-ui-ad-step-meta">{tileMeta(step, record, t)}</span>
        </li>
      ))}
    </ol>
  )
}
