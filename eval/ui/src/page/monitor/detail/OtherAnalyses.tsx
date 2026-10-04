// The other analyses of the turn the open one observed, and what changed between two of them. A turn that was
// analyzed again is read next to what it replaces: the rail lists them, Compare puts two side by side.
import { Button, Chip, Select } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { GitCompareArrows } from 'lucide-react'
import { type ReactNode, useEffect, useId, useMemo, useState } from 'react'
import type { EvalApi } from '../../../api'
import { statusPresentation } from '../../../model'
import type { AnalysisRecord, AnalysisResult } from '../../../types'
import { memberMeta, rowTime } from '../shell-state'
import { dayClock } from '../time'
import {
  canCompare,
  diffSignals,
  diffSuggestions,
  type Kept,
  keptNote,
  otherAnalyses,
  type Placed,
  readsNote,
  relation,
  signalLabel,
} from './compare'
import { Pill } from './marks'

/** Rows of the rail tile before "Show more". */
const SHOWN = 4

/** Every analysis of the open one's observed turn, newest first, or `null` until read (a failed read shows none). */
export function useTurnAnalyses(
  api: EvalApi,
  record: Pick<AnalysisRecord, 'observation_key' | 'updated_at'> | undefined,
  refreshKey: number,
): AnalysisRecord[] | null {
  const [rows, setRows] = useState<AnalysisRecord[] | null>(null)
  const key = record?.observation_key
  const updated = record?.updated_at
  // An analysis that moved on (`updated`), or a refresh, reads the turn again.
  useEffect(() => {
    if (!key) return
    let live = true
    api
      .list(key)
      .then((next) => {
        if (live) setRows(next)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [api, key, updated, refreshKey])
  return rows
}

/** The rail tile: one row per other analysis of the turn, each a link to it. */
export function OtherAnalyses({
  open,
  rows,
  now,
  comparing,
  onSelect,
  onCompare,
}: {
  open: AnalysisRecord
  rows: AnalysisRecord[] | null
  now: number
  /** The analysis the comparison panel shows. */
  comparing: string | null
  onSelect: (evaluationId: string) => void
  onCompare: (evaluationId: string) => void
}) {
  const headingId = useId()
  const [all, setAll] = useState(false)
  const others = useMemo(() => otherAnalyses(rows ?? [], open.evaluation_id), [rows, open.evaluation_id])
  if (others.length === 0) return null
  const shown = all ? others : others.slice(0, SHOWN)

  return (
    <section aria-labelledby={headingId} className="eval-ui-ad-tile eval-ui-oa">
      <div className="eval-ui-ad-tile-head">
        <h2 id={headingId} className="eval-ui-ad-h3">
          Other analyses of this turn
        </h2>
        <span className="eval-ui-ad-mono-quiet">{others.length}</span>
      </div>
      <ul className="eval-ui-oa-list">
        {shown.map((record) => {
          const status = statusPresentation(record)
          const note = relation(open, record)
          return (
            <li key={record.evaluation_id} className="eval-ui-oa-row">
              <button
                type="button"
                className="eval-ui-oa-open"
                aria-label={`Open the analysis of ${rowTime(record, now)}, ${status.label}`}
                onClick={() => onSelect(record.evaluation_id)}
              >
                <span className="eval-ui-oa-top">
                  <span className="eval-ui-ad-mono">{rowTime(record, now)}</span>
                  <Pill tone={status.tone}>{status.label}</Pill>
                </span>
                <span className="eval-ui-ad-mono-quiet">{memberMeta(record)}</span>
                {note ? <span className="eval-ui-ad-quiet eval-ui-oa-note">{note}</span> : null}
              </button>
              {canCompare(record) ? (
                <Button
                  variant="ghost"
                  size="sm"
                  className="eval-ui-oa-compare"
                  aria-pressed={comparing === record.evaluation_id}
                  onClick={() => onCompare(record.evaluation_id)}
                >
                  <GitCompareArrows size={16} aria-hidden="true" />
                  Compare
                </Button>
              ) : null}
            </li>
          )
        })}
      </ul>
      {others.length > SHOWN ? (
        <button type="button" className="eval-ui-oa-more" aria-expanded={all} onClick={() => setAll(!all)}>
          {all ? 'Show fewer' : `Show ${others.length - SHOWN} more`}
        </button>
      ) : null}
    </section>
  )
}

type Read = { status: 'loading' } | { status: 'error'; message: string } | { status: 'ready'; result: AnalysisResult }

function SuggestionRow({ tag, placed, children }: { tag: string; placed: Placed; children: ReactNode }) {
  return (
    <li className="eval-ui-cmp-item">
      <p className="eval-ui-cmp-title">
        <span className="eval-ui-ad-mono-quiet">{tag}</span>
        {placed.suggestion.title}
      </p>
      <Chip>{placed.suggestion.harness_component}</Chip>
      <p className="eval-ui-ad-quiet eval-ui-cmp-note">{children}</p>
    </li>
  )
}

function Group({
  label,
  tone,
  count,
  children,
}: {
  label: string
  tone: 'neutral' | 'warn' | 'ok'
  count: number
  children: ReactNode
}) {
  if (count === 0) return null
  return (
    <div className="eval-ui-cmp-group">
      <div className="eval-ui-cmp-group-head">
        <Pill tone={tone}>{label}</Pill>
        <span className="eval-ui-ad-mono-quiet">{count}</span>
      </div>
      <ul className="eval-ui-cmp-list">{children}</ul>
    </div>
  )
}

/**
 * Suggestions and signals of two analyses of one turn, the earlier as "before" and the later as "now": which
 * suggestions stayed, which are gone and which are new, and the same for signals by fingerprint. Close removes it.
 */
export function Comparison({
  api,
  current,
  otherId,
  rows,
  now,
  onPick,
  onClose,
}: {
  api: EvalApi
  current: AnalysisResult
  otherId: string
  rows: AnalysisRecord[] | null
  now: number
  onPick: (evaluationId: string) => void
  onClose: () => void
}) {
  const headingId = useId()
  const pickId = useId()
  const [read, setRead] = useState<Read>({ status: 'loading' })

  useEffect(() => {
    let live = true
    setRead({ status: 'loading' })
    api
      .result(otherId)
      .then((result) => {
        if (!live) return
        setRead(result ? { status: 'ready', result } : { status: 'error', message: 'That analysis no longer exists.' })
      })
      .catch((error: unknown) => {
        if (live) setRead({ status: 'error', message: errorMessage(error) })
      })
    return () => {
      live = false
    }
  }, [api, otherId])

  const other = read.status === 'ready' ? read.result : undefined
  const options = otherAnalyses(rows ?? [], current.record.evaluation_id)
    .filter(canCompare)
    .map((record) => ({
      value: record.evaluation_id,
      label: `${rowTime(record, now)} · ${statusPresentation(record).label} · ${memberMeta(record).split(' · ')[0]}`,
    }))

  const diff = useMemo(() => {
    if (!other) return undefined
    const [earlier, later] = other.record.created_at <= current.record.created_at ? [other, current] : [current, other]
    return {
      suggestions: diffSuggestions(
        earlier.assets.investigation?.suggestions ?? [],
        later.assets.investigation?.suggestions ?? [],
      ),
      signals: diffSignals(earlier.assets.snapshot?.diagnostics ?? [], later.assets.snapshot?.diagnostics ?? []),
    }
  }, [other, current])

  return (
    <section
      aria-labelledby={headingId}
      data-section="comparison"
      tabIndex={-1}
      className="eval-ui-ad-panel eval-ui-cmp"
    >
      <div className="eval-ui-cmp-head">
        <GitCompareArrows size={16} aria-hidden="true" />
        <h2 id={headingId} className="eval-ui-ad-h3">
          Compared with {otherId}
        </h2>
        {other ? <span className="eval-ui-ad-mono-quiet">{dayClock(other.record.created_at, now)}</span> : null}
        <Button variant="ghost" size="sm" className="eval-ui-cmp-close" onClick={onClose}>
          Close
        </Button>
      </div>
      <div className="eval-ui-cmp-pick">
        <label htmlFor={pickId} className="eval-ui-ad-quiet">
          Compare with
        </label>
        <Select<string> id={pickId} value={otherId} options={options} onChange={onPick} aria-label="Compare with" />
      </div>
      {read.status === 'loading' ? (
        <p className="eval-ui-ad-quiet" role="status">
          Reading the analysis…
        </p>
      ) : null}
      {read.status === 'error' ? (
        <p className="eval-ui-form-error" role="alert">
          Couldn't read that analysis. {read.message}
        </p>
      ) : null}
      {diff ? (
        <>
          <div className="eval-ui-cmp-summary">
            <Pill>{diff.suggestions.kept.length} kept</Pill>
            <Pill tone="warn">{diff.suggestions.dropped.length} dropped</Pill>
            <Pill tone="ok">{diff.suggestions.added.length} new</Pill>
          </div>
          <Group label="Kept" tone="neutral" count={diff.suggestions.kept.length}>
            {diff.suggestions.kept.map((kept: Kept) => (
              <SuggestionRow key={kept.now.index} tag={`S${kept.before.index} ↔ S${kept.now.index}`} placed={kept.now}>
                {keptNote(kept)}
              </SuggestionRow>
            ))}
          </Group>
          <Group label="Dropped" tone="warn" count={diff.suggestions.dropped.length}>
            {diff.suggestions.dropped.map((placed) => (
              <SuggestionRow key={placed.index} tag={`S${placed.index} · before`} placed={placed}>
                Not proposed again in the later analysis. The monitor doesn't know why.
              </SuggestionRow>
            ))}
          </Group>
          <Group label="New" tone="ok" count={diff.suggestions.added.length}>
            {diff.suggestions.added.map((placed) => (
              <SuggestionRow key={placed.index} tag={`S${placed.index} · now`} placed={placed}>
                {readsNote(placed.suggestion)}
              </SuggestionRow>
            ))}
          </Group>
          <div className="eval-ui-cmp-signals">
            <p className="eval-ui-cmp-signals-head">
              <strong>Signals</strong> <span className="eval-ui-ad-quiet">by fingerprint</span>{' '}
              <span className="eval-ui-ad-mono">
                {diff.signals.kept} kept · {diff.signals.dropped.length} dropped · {diff.signals.added.length} new
              </span>
            </p>
            {diff.signals.added.map((signal) => (
              <p key={`new-${signal.fingerprint}`} className="eval-ui-cmp-signal">
                <span className="eval-ui-cmp-tag">new</span>
                <span className="eval-ui-ad-mono">{signalLabel(signal)}</span>
              </p>
            ))}
            {diff.signals.dropped.map((signal) => (
              <p key={`dropped-${signal.fingerprint}`} className="eval-ui-cmp-signal">
                <span className="eval-ui-cmp-tag">dropped</span>
                <span className="eval-ui-ad-mono">{signalLabel(signal)}</span>
              </p>
            ))}
          </div>
          <p className="eval-ui-ad-quiet eval-ui-cmp-caption">
            Suggestions match when their Harness area and normalized title are the same. A rewording of one idea shows
            as one dropped and one new; the monitor doesn't read meaning. Failed analyses have no suggestions to
            compare, so Compare is absent for them.
          </p>
        </>
      ) : null}
    </section>
  )
}
