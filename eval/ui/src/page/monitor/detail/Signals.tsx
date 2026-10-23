// The deterministic findings, recorded by rules before any model ran. The
// analyst may add an advisory verdict; it never removes or hides a signal.
import { Chip } from '@iii-dev/console-ui'
import { ChevronDown, ChevronUp, Info, Minus } from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useId, useRef, useState } from 'react'
import { assessmentFor, shortHash, suggestionsUsing } from '../../../model'
import type { Diagnostic, Investigation, SignalVerdict, Snapshot } from '../../../types'
import { Disclosure, Inline, Pill, SectionHead } from './marks'
import { locateEntry, plural, probeParts, signalFooter, signalHolding } from './present'
import { scrollToElement } from './scroll'
import type { JumpTarget } from './shared'

const VERDICT: Record<SignalVerdict, { label: string; strong?: boolean; tone?: 'warn' }> = {
  worth_changing: { label: 'Worth changing', strong: true },
  likely_expected: { label: 'Likely expected' },
  unclear: { label: 'Unclear', tone: 'warn' },
}

function Probes({ probes }: { probes: Snapshot['excluded_probes'] }) {
  if (probes.length === 0) return null
  if (probes.length === 1) {
    const { call, note } = probeParts(probes[0])
    return (
      <div className="eval-ui-ad-probe">
        <Info size={16} aria-hidden="true" />
        <span>
          1 expected probe excluded: <code className="eval-ui-ad-code">{call}</code>
          {note ? ` ${note}` : ''}
        </span>
      </div>
    )
  }
  return (
    <Disclosure
      className="eval-ui-ad-probes"
      summary={(open) => (
        <>
          <Info size={16} aria-hidden="true" />
          <span className="eval-ui-ad-disclosure-title">{probes.length} expected probes excluded</span>
          {open ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}
        </>
      )}
    >
      <ul className="eval-ui-ad-probe-list">
        {probes.map((probe, index) => {
          const { call, note } = probeParts(probe)
          return (
            <li key={index}>
              <code className="eval-ui-ad-code">{call}</code>
              {note ? ` · ${note}` : ''}
            </li>
          )
        })}
      </ul>
    </Disclosure>
  )
}

function Assessment({
  diagnostic,
  investigation,
  terminal,
  used,
}: {
  diagnostic: Diagnostic
  investigation: Investigation | undefined
  /** The analysis has finished, so a missing investigation is final. */
  terminal: boolean
  used: ReactNode
}) {
  if (!investigation) {
    // Still running: the analyst may yet assess it. Finished without one: say so.
    if (!terminal) return null
    return (
      <div className="eval-ui-ad-assessment">
        <div className="eval-ui-ad-assessment-head">
          <span className="eval-ui-ad-minus">
            <Minus size={16} aria-hidden="true" />
            Not assessed.
          </span>
        </div>
        <p className="eval-ui-ad-quiet">
          No analyst investigation ran for this analysis, so the signal is kept exactly as the rule recorded it.
        </p>
      </div>
    )
  }
  const assessment = assessmentFor(diagnostic, investigation.signal_assessments)
  if (!assessment) {
    return (
      <div className="eval-ui-ad-assessment">
        <div className="eval-ui-ad-assessment-head">
          <span className="eval-ui-ad-minus">
            <Minus size={16} aria-hidden="true" />
            Not assessed.
          </span>
          {used}
        </div>
        <p className="eval-ui-ad-quiet">
          The analyst gave no verdict on this signal; it is kept exactly as the rule recorded it.
        </p>
      </div>
    )
  }
  const verdict = VERDICT[assessment.verdict]
  return (
    <div className="eval-ui-ad-assessment">
      <div className="eval-ui-ad-assessment-head">
        <span className="eval-ui-ad-label">Analyst assessment</span>
        <Pill tone={verdict.tone ?? 'neutral'} strong={verdict.strong}>
          {verdict.label}
        </Pill>
        {used}
      </div>
      <p className="eval-ui-ad-quiet">{assessment.explanation}</p>
    </div>
  )
}

export function Signals({
  snapshot,
  investigation,
  terminal,
  jump,
  narrow,
}: {
  snapshot: Snapshot
  investigation: Investigation | undefined
  terminal: boolean
  jump: JumpTarget | null
  narrow: boolean
}) {
  const headingId = useId()
  const section = useRef<HTMLElement>(null)
  const [flash, setFlash] = useState<{ fingerprint: string; n: number } | null>(null)
  const { diagnostics } = snapshot
  const dropped = diagnostics.length - snapshot.coverage.diagnostics_in_context

  // A chip whose entry is only evidence of a signal lands on that signal. Each
  // click is handled once: a reload that replaces the snapshot must not replay it.
  const handled = useRef(0)
  useEffect(() => {
    if (!jump || jump.n === handled.current) return
    handled.current = jump.n
    if (locateEntry(snapshot, jump.entry) !== 'signal') return
    const holder = signalHolding(snapshot, jump.entry)
    if (holder) setFlash({ fingerprint: holder.fingerprint, n: jump.n })
  }, [jump, snapshot])

  useEffect(() => {
    if (!flash) return
    const target = Array.from(section.current?.querySelectorAll<HTMLElement>('[data-fingerprint]') ?? []).find(
      (element) => element.dataset.fingerprint === flash.fingerprint,
    )
    scrollToElement(target)
    const timer = window.setTimeout(() => setFlash(null), 1800)
    return () => window.clearTimeout(timer)
  }, [flash])

  return (
    <section
      ref={section}
      aria-labelledby={headingId}
      className="eval-ui-ad-section"
      data-section="signals"
      tabIndex={-1}
    >
      <SectionHead
        id={headingId}
        title="Signals"
        meta={snapshot.rules_version}
        note="Recorded by rules before any model. Models can't remove them."
      />
      <div className="eval-ui-ad-panel eval-ui-ad-signals">
        {diagnostics.length === 0 ? <p className="eval-ui-ad-signal eval-ui-ad-quiet">No signals were found.</p> : null}
        {diagnostics.map((diagnostic) => {
          const usedBy = investigation ? suggestionsUsing(diagnostic, investigation.suggestions) : []
          const used =
            usedBy.length > 0 ? (
              <span className="eval-ui-ad-used">Used by {usedBy.map((n) => `S${n}`).join(', ')}</span>
            ) : null
          const inAssessment = narrow && investigation !== undefined
          return (
            <article
              key={diagnostic.fingerprint}
              className="eval-ui-ad-signal"
              data-fingerprint={diagnostic.fingerprint}
              data-flash={flash?.fingerprint === diagnostic.fingerprint || undefined}
              tabIndex={-1}
            >
              <div className="eval-ui-ad-signal-head">
                <span className="eval-ui-ad-mono-strong">{diagnostic.rule_id}</span>
                <Chip className="eval-ui-ad-mono-chip">{diagnostic.correlation}</Chip>
                {inAssessment ? null : used}
              </div>
              <p className="eval-ui-ad-text">
                <Inline text={diagnostic.observation} />
              </p>
              <Assessment
                diagnostic={diagnostic}
                investigation={investigation}
                terminal={terminal}
                used={inAssessment ? used : null}
              />
              <span className="eval-ui-ad-footer">{signalFooter(diagnostic, shortHash(diagnostic.fingerprint))}</span>
            </article>
          )
        })}
        {dropped > 0 ? (
          <div className="eval-ui-ad-probe">
            <Info size={16} aria-hidden="true" />
            <span>
              Only {snapshot.coverage.diagnostics_in_context} of {plural(diagnostics.length, 'signal')} fit in the
              context sent to the models. All of them are recorded here.
            </span>
          </div>
        ) : null}
        <Probes probes={snapshot.excluded_probes} />
      </div>
    </section>
  )
}
