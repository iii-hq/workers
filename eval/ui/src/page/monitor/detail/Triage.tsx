// What Jev, the triage judge, answered, and why the session was or was not
// sent to the analyst. Confidence is a spread of answers, not a verdict.
import { StatusPanel } from '@iii-dev/console-ui'
import { CircleAlert } from 'lucide-react'
import { useId } from 'react'
import { formatTokens, shortHash } from '../../../model'
import type { AnalysisAssets, AnalysisRecord, TriageFailure } from '../../../types'
import { SectionHead } from './marks'
import { choiceBars, plural, routingSentence, seconds, TRIAGE_QUESTION, triageChoice, twoDecimals } from './present'

function Failure({ failure }: { failure: TriageFailure }) {
  const { stats } = failure
  const facts = [
    failure.code,
    failure.http_status !== undefined ? `HTTP ${failure.http_status}` : undefined,
    `request ${failure.request_id}`,
  ]
    .filter(Boolean)
    .join(' · ')
  const used = stats
    ? [
        plural(stats.attempts, 'attempt'),
        `${formatTokens(stats.input_tokens + stats.output_tokens)} tokens${stats.usage_complete ? '' : ' (incomplete)'}`,
        seconds(stats.elapsed_ms),
      ].join(' · ')
    : undefined
  return (
    <StatusPanel
      variant="alert"
      role="alert"
      icon={<CircleAlert size={16} aria-hidden="true" />}
      headline="Triage did not return an answer"
      detail={
        <div className="eval-ui-ad-notice-body">
          <p>{failure.message}</p>
          <p className="eval-ui-ad-mono-quiet">{facts}</p>
          {used ? <p className="eval-ui-ad-mono-quiet">{used}</p> : null}
        </div>
      }
    />
  )
}

export function Triage({ record, assets }: { record: AnalysisRecord; assets: AnalysisAssets }) {
  const headingId = useId()
  const { triage, triage_failure: failure } = assets
  if (!triage && !failure) return null
  const answer = triageChoice(triage)
  const sentence = routingSentence(record, triage)
  const cut = sentence?.indexOf(': ') ?? -1

  return (
    <section aria-labelledby={headingId} className="eval-ui-ad-section">
      <SectionHead id={headingId} title="Triage" meta={`judge-typesafe${triage ? ` · ${triage.model}` : ''}`} />
      {failure ? (
        <Failure failure={failure} />
      ) : triage ? (
        <div className="eval-ui-ad-panel eval-ui-ad-triage">
          {answer ? (
            <>
              <div className="eval-ui-ad-triage-head">
                <span className="eval-ui-ad-mono-strong">{TRIAGE_QUESTION}</span>
                <span className="eval-ui-ad-quiet">choice</span>
                <span className="eval-ui-ad-mono eval-ui-ad-triage-confidence">
                  confidence {twoDecimals(answer.confidence)}
                </span>
              </div>
              <div className="eval-ui-ad-bars">
                {choiceBars(answer).map((bar) => (
                  <div key={bar.key} className="eval-ui-ad-bar-row" data-chosen={bar.chosen || undefined}>
                    <span className="eval-ui-ad-bar-name">{bar.key}</span>
                    <span className="eval-ui-ad-bar-track" aria-hidden="true">
                      <span
                        className="eval-ui-ad-bar-fill"
                        style={{ width: `${Math.round(bar.probability * 100)}%` }}
                      />
                    </span>
                    <span className="eval-ui-ad-bar-value">{twoDecimals(bar.probability)}</span>
                  </div>
                ))}
              </div>
            </>
          ) : (
            <p className="eval-ui-ad-quiet">
              Jev answered with a format this view doesn't draw ({Object.values(triage.answers)[0]?.type ?? 'no answer'}
              ).
            </p>
          )}
          <div className="eval-ui-ad-triage-notes">
            {sentence ? (
              cut > 0 ? (
                <span>
                  <strong className="eval-ui-ad-ink">{sentence.slice(0, cut)}:</strong> {sentence.slice(cut + 2)}
                </span>
              ) : (
                <span>{sentence}</span>
              )
            ) : null}
            <span>
              Confidence is the spread across these three answers. It is not the chance that a suggestion improves the
              Harness.
            </span>
            <span className="eval-ui-ad-mono-quiet">
              criteria {shortHash(triage.criteria_version)} · request {triage.request_id}
            </span>
          </div>
        </div>
      ) : null}
    </section>
  )
}
