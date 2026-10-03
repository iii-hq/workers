// The properties of the analysis, what the monitor itself cost, and the task's
// own metrics. Unknown values stay unknown: no cost is ever shown as $0.
import { formatDuration } from '@iii-dev/console-ui/format'
import type { ReactNode } from 'react'
import { Fragment, useId } from 'react'
import { formatCost, formatTokens, totalTokens } from '../../../model'
import type { AnalysisAssets, AnalysisRecord, MonitorUsage, Snapshot } from '../../../types'
import { count, formatCostShort, pathSegments, plural, sourceLine } from './present'

function Prop({ label, children, full }: { label: string; children: ReactNode; full?: boolean }) {
  return (
    <div className="eval-ui-ad-prop" data-full={full || undefined}>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

function Properties({
  record,
  assets,
  openInvestigation,
}: {
  record: AnalysisRecord
  assets: AnalysisAssets
  openInvestigation?: () => void
}) {
  const { snapshot, investigation } = assets
  const session = investigation?.session_id ?? record.analyst?.session_id
  const level = record.model.thinking_level ?? 'default'
  const ranAs =
    investigation?.effective_model &&
    (investigation.effective_model !== investigation.requested_model ||
      investigation.effective_provider !== investigation.requested_provider)
      ? `ran as ${investigation.effective_model} · ${investigation.effective_provider ?? 'unknown provider'}`
      : undefined
  return (
    <div className="eval-ui-ad-tile eval-ui-ad-props-tile">
      <dl className="eval-ui-ad-props">
        <Prop label="Origin">{record.origin === 'automatic' ? 'Automatic · turn completed' : 'Manual'}</Prop>
        {snapshot ? (
          <Prop label="Source">
            <span className="eval-ui-ad-mono">{sourceLine(snapshot)}</span>
          </Prop>
        ) : null}
        {snapshot ? (
          <Prop label="Task model" full>
            <span className="eval-ui-ad-mono">
              {snapshot.observed_model
                ? `${snapshot.observed_model} · ${snapshot.observed_provider ?? 'unknown provider'}`
                : 'not observed'}
            </span>
          </Prop>
        ) : null}
        <Prop label="Analyst" full>
          <span className="eval-ui-ad-mono">
            {record.model.model} · {record.model.provider}
          </span>
          <br />
          <span className="eval-ui-ad-quiet">thinking {level} · frozen at admission</span>
          {ranAs ? (
            <>
              <br />
              <span className="eval-ui-ad-quiet eval-ui-ad-mono">{ranAs}</span>
            </>
          ) : null}
        </Prop>
        <Prop label="Rules">
          <span className="eval-ui-ad-mono">{record.rules_version}</span>
        </Prop>
        {session ? (
          <Prop label="Investigation">
            {openInvestigation ? (
              <button type="button" className="eval-ui-ad-link" onClick={openInvestigation}>
                {session}
              </button>
            ) : (
              <span className="eval-ui-ad-mono">{session}</span>
            )}
          </Prop>
        ) : null}
        <Prop label="Directory" full>
          {record.code_root ? (
            <>
              <span className="eval-ui-ad-mono" title={record.code_root}>
                {pathSegments(record.code_root).map((part, index, parts) => (
                  <Fragment key={index}>
                    {part}
                    {index < parts.length - 1 ? <wbr /> : null}
                  </Fragment>
                ))}
              </span>
              <br />
              <span className="eval-ui-ad-quiet">frozen at admission</span>
            </>
          ) : (
            <span className="eval-ui-ad-quiet">Code access off</span>
          )}
        </Prop>
      </dl>
    </div>
  )
}

function Rows({ children }: { children: ReactNode }) {
  return <div className="eval-ui-ad-rows">{children}</div>
}

function Row({ label, value, sub }: { label: string; value: ReactNode; sub?: string }) {
  return (
    <div className="eval-ui-ad-row">
      <span>{label}</span>
      <span className="eval-ui-ad-mono eval-ui-ad-value">{value}</span>
      {sub ? <span className="eval-ui-ad-mono-quiet eval-ui-ad-row-sub">{sub}</span> : null}
    </div>
  )
}

/** `1 call · 1,204 tokens` — only what is known. */
function judgeSub(usage: MonitorUsage): string {
  if (usage.judge_calls === 0) return 'not called'
  const tokens = totalTokens(usage).judge
  return `${plural(usage.judge_calls, 'call')} · ${formatTokens(tokens)} tokens${usage.judge_usage_complete ? '' : ' (incomplete)'}`
}

function llmSub(usage: MonitorUsage, ran: boolean): string {
  const { llm } = totalTokens(usage)
  if (llm === undefined) return ran ? 'tokens not reported' : 'not run'
  return `${formatTokens(llm)} tokens`
}

function CostTile({ record, ran }: { record: AnalysisRecord; ran: boolean }) {
  const titleId = useId()
  const { usage } = record
  return (
    <section aria-labelledby={titleId} className="eval-ui-ad-tile">
      <div className="eval-ui-ad-tile-head">
        <h2 id={titleId} className="eval-ui-ad-h3">
          Monitor cost
        </h2>
        <span className="eval-ui-ad-mono eval-ui-ad-value">
          {usage.llm_cost_usd === undefined ? '—' : formatCost(usage.llm_cost_usd)}
        </span>
      </div>
      <Rows>
        <Row label="Triage · Jev" value="cost not reported" sub={judgeSub(usage)} />
        <Row
          label="Investigation · LLM"
          value={usage.llm_cost_usd === undefined && !ran ? '—' : formatCost(usage.llm_cost_usd)}
          sub={llmSub(usage, ran)}
        />
      </Rows>
      <p className="eval-ui-ad-quiet eval-ui-ad-foot">
        Counted apart from the task.
        {usage.judge_calls > 0 && usage.llm_cost_usd !== undefined
          ? " Jev doesn't report cost, so the total above is the investigation only."
          : ''}
      </p>
    </section>
  )
}

function MetricsTile({ snapshot }: { snapshot: Snapshot }) {
  const titleId = useId()
  const { totals, traces } = snapshot.metrics
  const tokens =
    totals.input_tokens === undefined && totals.output_tokens === undefined
      ? undefined
      : (totals.input_tokens ?? 0) + (totals.output_tokens ?? 0)
  return (
    <section aria-labelledby={titleId} className="eval-ui-ad-tile">
      <div className="eval-ui-ad-tile-head">
        <h2 id={titleId} className="eval-ui-ad-h3">
          Task metrics
        </h2>
        <span className="eval-ui-ad-mono-quiet">{snapshot.metrics_scope} · cumulative</span>
      </div>
      <Rows>
        <Row label="Calls" value={count(totals.function_calls)} />
        <Row label="Errors" value={count(totals.function_call_errors)} />
        <Row label="Tokens" value={formatTokens(tokens)} />
        {/* No traces means no duration was measured: never show it as 0 ms. */}
        <Row
          label="Duration"
          value={traces && traces.trace_count > 0 ? formatDuration(traces.duration_ms) : 'not reported'}
        />
        <Row label="Cost" value={formatCostShort(totals.cost_usd)} />
      </Rows>
      <p className="eval-ui-ad-quiet eval-ui-ad-foot">Whole session tree, not just this turn.</p>
    </section>
  )
}

export function Rail({
  record,
  assets,
  openInvestigation,
}: {
  record: AnalysisRecord
  assets: AnalysisAssets
  openInvestigation?: () => void
}) {
  const ran = Boolean(record.analyst || assets.investigation)
  return (
    <aside aria-label="Analysis properties" className="eval-ui-ad-rail">
      <Properties record={record} assets={assets} openInvestigation={openInvestigation} />
      <CostTile record={record} ran={ran} />
      {assets.snapshot ? <MetricsTile snapshot={assets.snapshot} /> : null}
    </aside>
  )
}
