// A validation this worker started, on the suggestion's card until it ends:
// one row per side with a segment per run. The sweep advances the state on the
// suggestion's row, so closing the page loses nothing and a second person sees
// the same thing. Runs that did not finish or did not start are named, never
// counted as done.
import { Button, CardHighlight, Chip, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { formatDuration } from '@iii-dev/console-ui/format'
import { CircleAlert, ExternalLink, FlaskConical, LoaderCircle, RefreshCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../../../api'
import type { ValidationRun } from '../../../types'
import { Pill } from './marks'
import { runActive } from './review-model'
import { classifyStartFailure, executionProgress, type Progress, sentence } from './start-validation-model'
import { formatClock } from './validation-view'

/** The row is read this often while the validation moves; the sweep itself runs every 15 s. */
const POLL_MS = 10_000
/** The E2E list is large: its progress is read less often. */
const PROGRESS_MS = 30_000

/** Follows a validation: the row's state while it is active, the E2E's per-run progress while it runs. */
function useFollow(api: EvalApi, evaluationId: string, initial: ValidationRun, index: number, onSettled: () => void) {
  const [run, setRun] = useState(initial)
  const [now, setNow] = useState(() => Date.now())
  const [progress, setProgress] = useState<{ baseline?: Progress; candidate?: Progress }>({})
  const settled = useRef(onSettled)
  settled.current = onSettled
  useEffect(() => setRun(initial), [initial])

  const active = runActive(run)
  const running = run.state === 'running'
  useEffect(() => {
    if (!active) return
    let alive = true
    const poll = async () => {
      if (document.hidden) return
      setNow(Date.now())
      try {
        const { reviews } = await api.reviews(evaluationId)
        const next = reviews.find((row) => row.suggestion_index === index)?.run
        if (!alive || !next) return
        setRun(next)
        if (!runActive(next)) settled.current()
      } catch {
        // The next tick asks again.
      }
    }
    const timer = window.setInterval(() => void poll(), POLL_MS)
    return () => {
      alive = false
      window.clearInterval(timer)
    }
  }, [active, api, evaluationId, index])

  const baselineId = run.baseline_execution_id
  const candidateId = run.candidate_execution_id
  useEffect(() => {
    if (!running) return
    let alive = true
    const read = async () => {
      if (document.hidden) return
      try {
        const list = await api.e2eExecutions([baselineId, candidateId].filter((id) => id !== undefined))
        if (alive)
          setProgress({
            baseline: executionProgress(list, baselineId),
            candidate: executionProgress(list, candidateId),
          })
      } catch {
        // Progress is a courtesy: the row's own state still says what is going on.
      }
    }
    void read()
    const timer = window.setInterval(() => void read(), PROGRESS_MS)
    return () => {
      alive = false
      window.clearInterval(timer)
    }
  }, [running, api, baselineId, candidateId])

  return { run, now, progress }
}

type Segment = 'done' | 'running' | 'waiting'

function Segments({ total, done, running }: { total: number; done: number; running: boolean }) {
  return (
    <span className="eval-ui-rv-segments" aria-hidden="true">
      {Array.from({ length: total }, (_, at) => {
        const kind: Segment = at < done ? 'done' : running && at === done ? 'running' : 'waiting'
        return <span key={at} data-kind={kind} />
      })}
    </span>
  )
}

function Side({
  label,
  commit,
  executionId,
  total,
  state,
  progress,
  elapsed,
  other,
}: {
  label: string
  commit: string
  executionId: string | undefined
  total: number
  state: ValidationRun['state']
  progress: Progress | undefined
  elapsed: string
  /** The other side's execution: a side that started while the other did not runs alone. */
  other: string | undefined
}) {
  const finished = state === 'finished' || state === 'attached'
  const failed = state === 'failed'
  const done = finished ? total : (progress?.finished ?? 0)
  const words = !executionId
    ? state === 'starting'
      ? 'Waiting for the E2E to accept'
      : "Didn't start"
    : failed
      ? other
        ? 'Started'
        : 'Started · keeps running in the E2E'
      : finished
        ? `Finished · ${total} of ${total} runs`
        : progress
          ? `Running · ${done} of ${progress.planned} runs`
          : 'Running'
  return (
    <div className="eval-ui-rv-side" data-lost={(!executionId && failed) || undefined}>
      <div className="eval-ui-rv-side-head">
        <span>
          <strong>{label}</strong> <span className="eval-ui-val-mono eval-ui-val-quiet">{commit.slice(0, 7)}</span>
        </span>
        <span
          className="eval-ui-rv-side-state"
          data-phase={executionId ? (failed ? 'started' : state) : failed ? 'lost' : 'starting'}
        >
          {words}
        </span>
      </div>
      <Segments
        total={progress?.planned ?? total}
        done={done}
        running={state === 'running' && executionId !== undefined}
      />
      {executionId ? (
        <span className="eval-ui-val-mono eval-ui-val-quiet eval-ui-val-small">
          {executionId.slice(0, 12)}
          {state === 'running' || state === 'starting' ? ` · ${elapsed}` : ''}
        </span>
      ) : null}
    </div>
  )
}

export function ValidationProgress({
  api,
  evaluationId,
  index,
  run: initial,
  readingPattern,
  narrow,
  onSettled,
  onStartAgain,
  onAttachOther,
  onOpenE2e,
}: {
  api: EvalApi
  evaluationId: string
  /** 0-based suggestion index. */
  index: number
  run: ValidationRun
  /** The signal the criterion counts, named while the transcripts are read. */
  readingPattern: string | undefined
  narrow: boolean
  /** The run left its active states (attached, or failed): read the analysis again. */
  onSettled: () => void
  onStartAgain: () => void
  onAttachOther: () => void
  onOpenE2e?: () => void
}) {
  const { run, now, progress } = useFollow(api, evaluationId, initial, index, onSettled)
  const size = narrow ? 'lg' : 'sm'
  const elapsed = formatDuration(Math.max(0, now - run.started_at))
  const failed = run.state === 'failed'
  // The backend's sentence, closed, so what follows it reads as a new one.
  const reason = sentence(run.error ? classifyStartFailure(run.error).text : 'The E2E did not report why')
  // The baseline started and the candidate did not (or the other way round): one runs alone.
  const half = failed && Boolean(run.baseline_execution_id) !== Boolean(run.candidate_execution_id)
  const pill =
    run.state === 'starting' ? (
      <Pill tone="accent">
        <LoaderCircle size={16} className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
        Starting
      </Pill>
    ) : run.state === 'running' ? (
      <Pill tone="accent" pulse>
        Running
      </Pill>
    ) : run.state === 'finished' ? (
      <Pill tone="ok">Both finished</Pill>
    ) : failed ? (
      <Pill tone={half ? 'warn' : 'alert'}>{half ? 'Half started' : 'Failed'}</Pill>
    ) : (
      <Pill tone="ok">Attached</Pill>
    )

  return (
    <CardHighlight className="eval-ui-val-panel" role="region" aria-label="E2E validation">
      <div className="eval-ui-val-head">
        <h4 className="eval-ui-val-heading">E2E validation</h4>
        <Chip className="eval-ui-val-mono">{run.scenario_id}</Chip>
        {pill}
        <span className="eval-ui-val-quiet eval-ui-val-earlier">started {formatClock(run.started_at)}</span>
      </div>
      <Side
        label="Baseline"
        commit={run.baseline_commit}
        executionId={run.baseline_execution_id}
        total={run.runs}
        state={run.state}
        progress={progress.baseline}
        elapsed={elapsed}
        other={run.candidate_execution_id}
      />
      <Side
        label="Candidate"
        commit={run.candidate_commit}
        executionId={run.candidate_execution_id}
        total={run.runs}
        state={run.state}
        progress={progress.candidate}
        elapsed={elapsed}
        other={run.baseline_execution_id}
      />

      {run.state === 'starting' ? (
        <p className="eval-ui-val-note">Asking the E2E to start both executions. This takes a few seconds.</p>
      ) : run.state === 'running' ? (
        <p className="eval-ui-val-note">
          Docker builds the Harness from GitHub at each commit; a commit that was built before is reused. The runs
          attach to this suggestion when both finish.
        </p>
      ) : run.state === 'finished' ? (
        <p className="eval-ui-val-line" role="status">
          <LoaderCircle className={`${uiClasses.icon} ${uiClasses.spin}`} aria-hidden />
          <span>
            Reading the transcripts{readingPattern ? ' and counting ' : ' and attaching the runs'}
            {readingPattern ? <span className="eval-ui-val-mono">{readingPattern}</span> : null}…
            {run.error ? ` ${sentence(classifyStartFailure(run.error).text)}` : ''}
          </span>
        </p>
      ) : null}

      {failed ? (
        <StatusPanel
          variant={half ? 'warn' : 'alert'}
          role="alert"
          icon={<CircleAlert className={uiClasses.icon} aria-hidden />}
          headline={half ? 'One execution did not start' : "The validation didn't complete"}
          detail={
            <>
              {reason}{' '}
              {half
                ? 'The execution that started keeps running in the E2E; stopping it is done there. Nothing is attached until you attach a pair.'
                : 'Nothing was attached.'}
            </>
          }
        />
      ) : null}

      {failed || onOpenE2e ? (
        <div className="eval-ui-rv-inline-actions">
          {failed ? (
            <>
              <Button variant="pill" size={size} onClick={onStartAgain}>
                <RefreshCw size={16} aria-hidden="true" />
                Start again
              </Button>
              <Button variant="ghost" size={size} onClick={onAttachOther}>
                <FlaskConical size={16} aria-hidden="true" />
                Attach other runs
              </Button>
            </>
          ) : null}
          {onOpenE2e ? (
            <Button variant="ghost" size={size} onClick={onOpenE2e}>
              Open in E2E
              <ExternalLink size={16} aria-hidden="true" />
            </Button>
          ) : null}
        </div>
      ) : null}
    </CardHighlight>
  )
}
