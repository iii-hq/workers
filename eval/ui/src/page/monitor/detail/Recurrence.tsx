// "After release": once a suggestion is shipped with a Harness version, the
// monitor counts the suggestion's patterns per analysis on the versions before
// it and from it on. A drop is evidence, not proof of the pull request; a
// pattern that is still there is a lead, never a failure.
import { Button, CardHighlight, Chip, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Clock, Plus, TriangleAlert } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { EvalApi } from '../../../api'
import { formatStamp } from '../../../model'
import type { Recurrence as RecurrenceData, RecurrencePattern, SuggestionReview } from '../../../types'
import { Pill } from './marks'
import { TransitionDialog } from './ReviewStatus'
import { lifecycleBadge, patternParts, readPattern } from './review-model'

/** Analyses a side needs before the two are compared. */
const MIN_ANALYSES = 5

type Load = { phase: 'loading' } | { phase: 'failed'; message: string } | { phase: 'ready'; data: RecurrenceData }

const per = (pattern: RecurrencePattern | undefined) =>
  pattern?.per_analysis === undefined ? '—' : pattern.per_analysis.toFixed(1)

function Bars({
  version,
  before,
  after,
  analysesBefore,
  analysesAfter,
}: {
  version: string
  before: RecurrencePattern | undefined
  after: RecurrencePattern | undefined
  analysesBefore: number
  analysesAfter: number
}) {
  const largest = Math.max(before?.per_analysis ?? 0, after?.per_analysis ?? 0)
  const bar = (label: string, pattern: RecurrencePattern | undefined, n: number, kind: 'before' | 'after') => (
    <div className="eval-ui-rv-bar" data-kind={kind}>
      <span className="eval-ui-val-mono">{label}</span>
      <span className="eval-ui-rv-bar-track" aria-hidden="true">
        <span style={{ width: largest > 0 ? `${((pattern?.per_analysis ?? 0) / largest) * 100}%` : '0%' }} />
      </span>
      <span className="eval-ui-val-mono">
        {per(pattern)} <span className="eval-ui-val-quiet">n={n}</span>
      </span>
    </div>
  )
  return (
    <div className="eval-ui-rv-bars">
      {bar(`< ${version}`, before, analysesBefore, 'before')}
      {bar(`≥ ${version}`, after, analysesAfter, 'after')}
    </div>
  )
}

export function RecurrencePanel({
  api,
  evaluationId,
  review,
  title,
  narrow,
  onSaved,
}: {
  api: EvalApi
  evaluationId: string
  review: SuggestionReview
  title: string
  narrow: boolean
  onSaved: (row: SuggestionReview) => void
}) {
  const { lifecycle } = review
  const version = lifecycle.version
  const [load, setLoad] = useState<Load>({ phase: 'loading' })
  const [adding, setAdding] = useState(false)

  const counted = review.patterns.length > 0
  useEffect(() => {
    if (!version || !counted) return
    let alive = true
    setLoad({ phase: 'loading' })
    api.recurrence(evaluationId, review.suggestion_index).then(
      (data) => alive && setLoad({ phase: 'ready', data }),
      (cause) =>
        alive &&
        setLoad({ phase: 'failed', message: errorMessage(cause).replace(/^.*recurrence_unavailable:\s*/, '') }),
    )
    return () => {
      alive = false
    }
  }, [api, evaluationId, review.suggestion_index, version, counted])

  const shippedAt = [...lifecycle.history].reverse().find((event) => event.status === 'shipped')?.at
  const badge = lifecycleBadge(lifecycle)
  const target = { api, evaluationId, index: review.suggestion_index, title }

  const head = (
    <div className="eval-ui-val-head">
      <h4 className="eval-ui-val-heading">After release</h4>
      <Pill tone={badge.tone}>{badge.label}</Pill>
      {shippedAt ? (
        <span className="eval-ui-val-quiet eval-ui-val-earlier">marked shipped {formatStamp(shippedAt)}</span>
      ) : null}
    </div>
  )

  let body: React.ReactNode
  if (!version) {
    body = (
      <StatusPanel
        variant="info"
        icon={<Plus className={uiClasses.icon} aria-hidden />}
        headline="Add the Harness version it shipped in"
        detail="Without it the monitor can't split analyses into before and after. The version is the one in the Harness package, such as 1.8.44."
        action={
          <Button type="button" variant="pill" size={narrow ? 'lg' : 'sm'} onClick={() => setAdding(true)}>
            Add version
          </Button>
        }
      />
    )
  } else if (review.patterns.length === 0) {
    // Counted per pattern: a suggestion that cites no signal has nothing to count before and after.
    body = (
      <StatusPanel
        variant="info"
        icon={<Clock className={uiClasses.icon} aria-hidden />}
        headline="Nothing to count after the release"
        detail="This suggestion cites no signal the monitor counts, so the release can't be measured here. Validate it in E2E with a measure criterion instead."
      />
    )
  } else if (load.phase === 'loading') {
    body = <p className="eval-ui-val-note">Counting the analyses on each side of v{version}…</p>
  } else if (load.phase === 'failed') {
    body = (
      <StatusPanel
        variant="warn"
        role="status"
        icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
        headline="The monitor couldn't compare the versions"
        detail={load.message}
      />
    )
  } else {
    const { data } = load
    const patterns = review.patterns
    const find = (list: RecurrencePattern[], pattern: string) => list.find((item) => item.pattern === pattern)
    const few = Math.min(data.before.analyses, data.from_version.analyses) < MIN_ANALYSES
    const readings = patterns.map((pattern) => ({
      pattern,
      before: find(data.before.patterns, pattern),
      after: find(data.from_version.patterns, pattern),
    }))
    const still = readings.find((item) => !few && readPattern(item.before, item.after) === 'still')
    body = (
      <>
        {readings.map(({ pattern, before, after }) => {
          const { rule, target: on } = patternParts(pattern)
          return (
            <div key={pattern} className="eval-ui-rv-pattern">
              <div className="eval-ui-val-row">
                <span className="eval-ui-val-quiet eval-ui-val-small">Pattern</span>
                <Chip className="eval-ui-val-mono">{on ? `${rule} · ${on}` : rule}</Chip>
              </div>
              <p className="eval-ui-val-evidence-line">
                <span className="eval-ui-val-mono eval-ui-val-ink">{rule}</span> per analysis:{' '}
                <strong className="eval-ui-val-evidence-figure">
                  {per(before)} on &lt; {version} (n={data.before.analyses}) → {per(after)} on ≥ {version} (n=
                  {data.from_version.analyses})
                </strong>
              </p>
              <Bars
                version={version}
                before={before}
                after={after}
                analysesBefore={data.before.analyses}
                analysesAfter={data.from_version.analyses}
              />
            </div>
          )
        })}
        {few ? (
          <StatusPanel
            variant="info"
            icon={<Clock className={uiClasses.icon} aria-hidden />}
            headline="Not enough yet"
            detail={`${data.before.analyses} analyses before and ${data.from_version.analyses} from ${version}; the panel compares from ${MIN_ANALYSES} on each side. It updates as the monitor analyses more sessions.`}
          />
        ) : null}
        {still?.after ? (
          <StatusPanel
            variant="warn"
            role="status"
            icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
            headline="The signal is still there after the release"
            detail={`It was seen in ${still.after.analyses_with} of the ${data.from_version.analyses} analyses from ${version}. Check the analyses before concluding the change missed, since the pull request may not cover this pattern's target.`}
          />
        ) : null}
        {data.without_version > 0 ? (
          <StatusPanel
            variant="warn"
            role="status"
            icon={<TriangleAlert className={uiClasses.icon} aria-hidden />}
            headline={`${data.without_version} ${data.without_version === 1 ? 'analysis has' : 'analyses have'} no Harness version`}
            detail="Manual analyses and reanalyses (they may be of an older session), analyses admitted before the monitor recorded the version, and ones admitted while the engine didn't answer are left out of both sides, not guessed."
          />
        ) : null}
        <p className="eval-ui-val-note">
          Per analysis, not per hour: each analysis is one observed turn.{' '}
          {data.before.analyses + data.from_version.analyses} analyses with a recorded Harness version. Other changes in
          v{version} can explain a drop too; this panel is evidence, not proof of the pull request.
        </p>
      </>
    )
  }

  return (
    <div className="eval-ui-val" data-narrow={narrow || undefined}>
      <CardHighlight className="eval-ui-val-panel" role="region" aria-label="After release">
        {head}
        {body}
      </CardHighlight>
      {adding ? (
        <TransitionDialog
          target={target}
          kind="shipped"
          lifecycle={lifecycle}
          narrow={narrow}
          onClose={() => setAdding(false)}
          onSaved={onSaved}
        />
      ) : null}
    </div>
  )
}
