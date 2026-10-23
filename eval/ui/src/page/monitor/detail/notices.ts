// The notice that opens an analysis detail in every state but "completed with
// suggestions": what the monitor is doing, or exactly why it stopped. Failures
// never read like a healthy result, and "completed" never speaks for the task.
import { formatBytes, formatDuration } from '@iii-dev/console-ui/format'
import { formatCost, formatTokens, remainingMs, totalTokens } from '../../../model'
import type {
  AnalysisRecord,
  AnalysisResult,
  AnalysisStatus,
  Failure,
  MonitorLimits,
  MonitorUsage,
} from '../../../types'
import { span } from '../time'
import {
  capitalize,
  clock,
  count,
  formatCostShort,
  investigationCaps,
  parseSizes,
  plural,
  routingSentence,
  seconds,
  shortId,
  triageChoice,
  twoDecimals,
} from './present'

export type NoticeTone = 'info' | 'running' | 'warn' | 'alert'
export type NoticeIcon = 'spinner' | 'clock' | 'alert' | 'help' | 'warn' | 'ban' | 'check' | 'file-x'
/**
 * What a notice offers besides the masthead's one Reanalyze: stop the run, read the signals, or the one thing the
 * cause needs done outside the monitor (`billing`, `judge-settings`) or in the analyst's session.
 */
export type NoticeAction = 'cancel' | 'signals' | 'session' | 'billing' | 'judge-settings'

/**
 * Failures a new run would hit again: `eval::analyze` refuses a descendant
 * session and the monitor's own sessions are never analyzed.
 */
const NOT_REANALYZABLE = new Set(['not_a_root_session', 'monitor_session'])

export function canReanalyze(record: Pick<AnalysisRecord, 'failure'>): boolean {
  return !(record.failure && NOT_REANALYZABLE.has(record.failure.code))
}

export interface StateCopy {
  tone: NoticeTone
  icon: NoticeIcon
  title: string
  /** Sentences, in plain copy. */
  body: string
  /** Backend wording kept quiet underneath, for failures. */
  message?: string
  /** One mono line: the code, a time, the request id. */
  detail?: string
  /** A new run is possible: the notice ends with what it would cost. */
  estimate?: boolean
  actions: NoticeAction[]
}

function stageWord(stage: AnalysisStatus | undefined): string {
  switch (stage) {
    case 'judging':
      return 'triage'
    case 'investigating':
      return 'the investigation'
    case 'collecting':
    case 'queued':
      return 'collection'
    default:
      return 'an unknown stage'
  }
}

/** Known usage only: unknown cost is said so, never `$0`. */
export function usageSummary(usage: MonitorUsage): string {
  const tokens = totalTokens(usage)
  const used = tokens.judge + (tokens.llm ?? 0)
  const parts: string[] = []
  if (used === 0 && usage.judge_calls === 0 && tokens.llm === undefined) return 'No model was called'
  parts.push(
    `${formatTokens(used)} tokens${usage.judge_calls > 0 && !usage.judge_usage_complete ? ' (triage usage incomplete)' : ''}`,
  )
  parts.push(usage.llm_cost_usd === undefined ? 'cost not reported' : `${formatCost(usage.llm_cost_usd)} LLM`)
  return parts.join(' · ')
}

/** `1 step at most, up to 200,000 tokens.`; the caps this analysis was admitted with. */
function investigationBudget(limits: MonitorLimits | undefined, codeAccess: boolean): string {
  if (!limits) return 'The analyst works within its step and token budget.'
  const caps = investigationCaps(limits, codeAccess)
  return `${plural(caps.steps, 'step')} at most, up to ${count(caps.totalTokens)} tokens.`
}

function pendingCopy(reason: string): { title: string; body: string } {
  if (reason.includes('descendant')) {
    return {
      title: 'Waiting for the session tree to finish',
      body: 'A descendant session is still running, or its metrics are incomplete.',
    }
  }
  if (reason.includes('definitive end')) {
    return {
      title: 'Waiting for the session turn to end',
      body: "The observed turn hasn't reached a definitive end yet.",
    }
  }
  if (reason.includes('incomplete or inconsistent')) {
    return {
      title: 'Waiting for the session tree to settle',
      body: 'The session tree is incomplete or inconsistent right now.',
    }
  }
  return { title: 'Collection is waiting', body: 'The monitor is waiting before it reads the session.' }
}

const KEPT = 'The signals and the snapshot are kept.'

/** The provider's own words, when they fit in a sentence; longer ones stay in the quiet line. */
function providerWords(message: string): string | undefined {
  const words = message.trim().replace(/[.\s]+$/, '')
  return words && words.length <= 160 && !words.includes('\n') ? words : undefined
}

function judgeCopy(
  code: string,
  limits: MonitorLimits | undefined,
  assets: { message: string; httpStatus?: number },
): { title: string; body: string; inlined?: boolean } {
  switch (code) {
    case 'judge_out_of_credits': {
      const words = providerWords(assets.message)
      return {
        title: 'Jev is out of credits',
        body: `judge-typesafe answered HTTP ${assets.httpStatus ?? 402}${words ? `: ${words}` : ''}. Triage can't run until credits are added, so no analyst model was called and nothing was spent on this analysis. ${KEPT}`,
        inlined: words !== undefined,
      }
    }
    case 'judge_provider_unavailable':
      return {
        title: 'judge-typesafe is unavailable',
        body: `The provider didn't answer. ${KEPT} No other provider or model was tried.`,
      }
    case 'judge_missing_key':
      return {
        title: 'Jev has no API key',
        body: `judge-typesafe has no key, so triage can't run. Add the key in the judge-typesafe settings, then reanalyze. ${KEPT}`,
      }
    case 'judge_deadline':
      return {
        title: 'Triage ran out of time',
        body: `Jev didn't answer within its ${limits ? `${seconds(limits.judge_timeout_ms, 0)} ` : 'time '}limit. ${KEPT} The call wasn't repeated.`,
      }
    case 'judge_bus':
      return {
        title: "Triage couldn't reach judge-typesafe",
        body: `The call failed before the provider answered. ${KEPT}`,
      }
    case 'judge_invalid_response':
      return {
        title: 'Triage returned an unusable answer',
        body: `The answer didn't match the question's format, so the monitor discarded it instead of guessing. ${KEPT}`,
      }
    case 'judge_cancelled':
      return { title: 'Triage was cancelled', body: `The Jev call ended before it answered. ${KEPT}` }
    default:
      return {
        title: 'Triage failed',
        body: `judge-typesafe answered ${code.slice('judge_'.length).replaceAll('_', ' ')}. ${KEPT}`,
      }
  }
}

/** Steps an analyst may take: what the backend named in its message (`(32)`), else the cap of the analysis. */
function stepsOf(record: Pick<AnalysisRecord, 'code_root'>, failure: Failure, limits: MonitorLimits | undefined) {
  const named = /\((\d+)\)/.exec(failure.message)
  if (named) return Number(named[1])
  return limits ? investigationCaps(limits, Boolean(record.code_root)).steps : undefined
}

/** `417k`, `1,204`: tokens as a budget reads. */
function tokensOf(tokens: number): string {
  return tokens >= 10_000 ? `${Math.round(tokens / 1000)}k` : count(tokens)
}

/** What an analysis that ran a model spent, as far as the model's provider reported it; empty when nothing ran. */
function spentClause(usage: MonitorUsage): string | undefined {
  if (usage.llm_cost_usd !== undefined) return `${formatCostShort(usage.llm_cost_usd)} was spent`
  return totalTokens(usage).llm === undefined ? undefined : "its cost wasn't reported"
}

/**
 * The failure's code, with the one rewrite old records need: before `judge_out_of_credits` existed, no credits was a
 * `judge_http` whose message (and triage failure) still say HTTP 402.
 */
export function failureCode(failure: Failure, httpStatus?: number): string {
  const credits = failure.code === 'judge_http' && (httpStatus === 402 || /\bHTTP 402\b/.test(failure.message))
  return credits ? 'judge_out_of_credits' : failure.code
}

/** The headline of a failure; the list says it too, so it is one string in one place. */
export function failureHeadline(failure: Failure, steps?: number): string {
  const { stage } = failure
  const code = failureCode(failure)
  if (code.startsWith('judge_')) return judgeCopy(code, undefined, { message: '' }).title
  switch (code) {
    case 'deadline':
      return 'The analysis ran out of time'
    case 'source_not_found':
      return 'The session has no turn record'
    case 'source_advanced':
    case 'inconsistent_evidence':
      return 'The session changed during capture'
    case 'evidence_unreadable':
      return "The evidence couldn't be read"
    case 'not_a_root_session':
      return 'This is not a root session'
    case 'monitor_session':
      return "The monitor doesn't analyze its own sessions"
    case 'coverage_insufficient':
      return stage === 'investigating' ? 'Evidence too large for the analyst' : 'Evidence too large to analyze'
    case 'snapshot_missing':
      return 'The saved capture is missing'
    case 'external_outcome_unknown':
      return stage === 'investigating' ? 'Investigation result is unknown' : 'Triage result is unknown'
    case 'analyst_rejected':
      return "Harness didn't accept the investigation"
    case 'analyst_failed':
      return 'The analyst ended without a result'
    case 'analyst_turn_changed':
      return 'The investigation session changed'
    case 'analyst_step_cap':
      return steps === undefined
        ? 'The analyst used all its steps'
        : steps === 1
          ? 'The analyst used its only step'
          : `The analyst used all ${steps} steps`
    case 'analyst_output_invalid':
      return "The analyst's answer was unusable"
    case 'cost_cap':
      return 'The daily cost cap was reached'
    default:
      return 'The analysis failed'
  }
}

function failureCopy(result: AnalysisResult, limits: MonitorLimits | undefined): StateCopy | null {
  const { record, assets } = result
  const failure = record.failure
  if (!failure) return null
  const { stage } = failure
  const status = assets.triage_failure?.http_status
  const code = failureCode(failure, status)
  const at = record.completed_at ?? record.updated_at
  const signals = record.counters.diagnostics
  const keptSignals =
    signals > 0 ? ` ${plural(signals, 'signal')} from the capture ${signals === 1 ? 'is' : 'are'} kept.` : ''
  const request = record.judge_call?.request_id
  const spent = spentClause(record.usage)
  const base = {
    tone: 'alert' as NoticeTone,
    icon: 'alert' as NoticeIcon,
    message: failure.message || undefined,
    detail: [
      failure.code,
      code.startsWith('judge_') && status !== undefined ? `HTTP ${status}` : undefined,
      clock(at),
      stage === 'judging' && request ? `request ${request}` : undefined,
    ]
      .filter(Boolean)
      .join(' · '),
    estimate: canReanalyze(record),
    actions: [] as NoticeAction[],
  }
  const title = failureHeadline({ ...failure, code }, stepsOf(record, failure, limits))
  const copy = (body: string, extra: Partial<StateCopy> = {}): StateCopy => ({ ...base, title, body, ...extra })
  /** The analyst's session holds what it did: the one thing to read when it ended badly. */
  const analyst: Partial<StateCopy> = { actions: ['session'] }
  const noEstimate = { estimate: false, actions: [] as NoticeAction[] }

  if (code.startsWith('judge_')) {
    const judge = judgeCopy(code, limits, { message: failure.message, httpStatus: status })
    const action: NoticeAction[] =
      code === 'judge_out_of_credits' ? ['billing'] : code === 'judge_missing_key' ? ['judge-settings'] : []
    return copy(judge.body, { actions: action, ...(judge.inlined ? { message: undefined } : {}) })
  }
  switch (code) {
    case 'deadline':
      return copy(
        `It didn't finish within ${seconds(record.deadline - record.created_at, 0)}. It was stopped during ${stageWord(stage)}.${assets.snapshot ? ' The evidence captured so far is kept.' : ''}`,
        { icon: 'clock' },
      )
    case 'source_not_found':
      return copy(
        'Harness has no turn record for this session, so there was nothing to capture. It may have been deleted. No model was called.',
        { icon: 'file-x' },
      )
    case 'source_advanced':
      return copy(
        `A newer turn started while turn ${shortId(record.turn_id)} was being read. Turns weren't mixed, so the capture was stopped. Reanalyze once the session settles.`,
      )
    case 'inconsistent_evidence':
      return copy(
        "The observed turn resumed, or a descendant session changed, while the evidence was being read. Turns weren't mixed, so the capture was stopped. Reanalyze once the session settles.",
      )
    case 'evidence_unreadable':
      return copy(
        'A page of the transcript was missing or malformed, so the monitor stopped instead of skipping evidence. No model was called.',
        { icon: 'file-x' },
      )
    case 'not_a_root_session':
      return copy(
        "It belongs to another session's tree. Analyze its root session instead: descendants are analyzed together with their root.",
        { tone: 'warn', icon: 'warn', ...noEstimate },
      )
    case 'monitor_session':
      return copy(
        "This session was created by the monitor to investigate another analysis. It's never analyzed, so analyses can't feed each other.",
        { tone: 'warn', icon: 'warn', ...noEstimate },
      )
    case 'coverage_insufficient': {
      const sizes = parseSizes(failure.message)
      const forContext = stage === 'investigating'
      const measured = sizes
        ? forContext
          ? `The model context is ${formatBytes(sizes.size)}; the limit is ${formatBytes(sizes.limit)}.`
          : `The snapshot needs ${formatBytes(sizes.size)}; the limit is ${formatBytes(sizes.limit)}.`
        : 'The evidence is over the size the monitor can analyze.'
      return copy(
        `${measured} ${forContext ? 'The analyst model was not called.' : 'No model was called.'}${keptSignals} This doesn't mean the session was healthy.`,
        {
          tone: 'warn',
          icon: 'warn',
          message: sizes ? undefined : base.message,
          actions: signals > 0 ? ['signals'] : [],
        },
      )
    }
    case 'snapshot_missing':
      return copy(
        "The evidence captured for this analysis can't be found, so the next stage couldn't run. Reanalyze to capture the session again.",
        { icon: 'file-x' },
      )
    case 'external_outcome_unknown':
      return copy(
        stage === 'investigating'
          ? "The eval worker restarted after the request to the analyst was sent, and no result was saved. The request wasn't repeated, so it can't be charged twice."
          : "The eval worker restarted after the Jev call started, and no answer was saved. The call wasn't repeated, so it can't be charged twice.",
        { icon: 'help' },
      )
    case 'analyst_rejected':
      return copy(
        "The request to start the analyst's turn was refused, so there are no suggestions. Triage and signals are kept.",
      )
    case 'analyst_failed': {
      const started = record.stages.find((stage) => stage.status === 'investigating')?.at
      const after = started === undefined ? '' : ` after ${span(at - started)}`
      return copy(
        `The analyst's turn ended${after} before it delivered a result, so there is nothing to show.${record.analyst ? ' What it did is kept in the analyst session.' : ''}${spent ? ` ${capitalize(spent)}.` : ''} Triage and signals are kept.`,
        analyst,
      )
    }
    case 'analyst_turn_changed':
      return copy(
        "Another turn started in the investigation session, so its result can't be trusted. Triage and signals are kept.",
        analyst,
      )
    case 'analyst_step_cap': {
      const steps = stepsOf(record, failure, limits)
      const caps = limits ? investigationCaps(limits, Boolean(record.code_root)) : undefined
      const tokens = totalTokens(record.usage).llm
      const used = [
        steps === undefined ? undefined : `${steps} of ${steps} steps`,
        tokens === undefined
          ? undefined
          : `${tokensOf(tokens)}${caps ? ` of ${tokensOf(caps.totalTokens)}` : ''} tokens`,
        spent,
      ].filter(Boolean)
      return copy(
        `It reached the step cap before delivering a result, so there is no suggestion to show.${used.length ? ` ${used.join(' · ')}.` : ''} The cap is fixed in this version. Triage and signals are kept.`,
        analyst,
      )
    }
    case 'analyst_output_invalid':
      return copy(
        "The investigation finished, but its result doesn't match the suggestion format, so nothing is shown as a suggestion. Triage and signals are kept.",
        analyst,
      )
    case 'cost_cap':
      return copy(
        "The day's cost cap was reached while this analysis waited, so the analyst model was not called and nothing was spent on it. Triage and signals are kept. Raise the cap in Settings, or reanalyze: a manual analysis is never held back by the cap, and it investigates if Jev answers needs_investigation again.",
        { tone: 'warn', icon: 'warn' },
      )
    default:
      return copy(`It stopped during ${stageWord(stage)}.`, { message: failure.message || undefined })
  }
}

function completedCopy(result: AnalysisResult): StateCopy | null {
  const { record, assets } = result
  if (record.counters.suggestions > 0) return null
  // Every proposal was rejected: the Suggestions section says so.
  if (record.counters.rejected_suggestions > 0) return null
  const signals = record.counters.diagnostics
  const answer = triageChoice(assets.triage)
  const parts = [signals === 0 ? 'No signals.' : `${plural(signals, 'signal')} recorded.`]
  if (answer) parts.push(`Triage said ${answer.choice} at ${twoDecimals(answer.confidence)}.`)
  const skipped = record.routing && !record.routing.investigate
  if (skipped) parts.push('It was not sent to investigation.')
  else if (record.routing) parts.push('The analyst investigated and proposed no change.')
  // "Nothing worth changing" is only said of a capture that was complete: a
  // missing signal in a partial one is not evidence of healthy behavior, and a
  // signal that no analyst looked at is not one either.
  const unexamined = signals > 0 && skipped
  const coverage = assets.snapshot?.coverage.level ?? record.coverage
  if (coverage === 'complete' && !unexamined) {
    return {
      tone: 'info',
      icon: 'check',
      title: 'Nothing worth changing was found',
      body: parts.join(' '),
      actions: [],
    }
  }
  if (coverage === 'insufficient' || coverage === 'partial') {
    parts.push(`The capture was ${coverage}, so this doesn't mean the session was healthy.`)
  }
  return {
    tone: coverage === 'insufficient' ? 'warn' : 'info',
    icon: coverage === 'insufficient' ? 'warn' : 'help',
    title: unexamined ? 'Signals recorded, not investigated' : 'No suggestions were made',
    body: parts.join(' '),
    actions: [],
  }
}

/** The notice for a result, or `null` when the suggestions are the story. */
export function describeState(
  result: AnalysisResult,
  now: number,
  limits: MonitorLimits | undefined,
): StateCopy | null {
  const { record, assets } = result
  const signals = record.counters.diagnostics
  switch (record.status) {
    case 'queued':
      return {
        tone: 'info',
        icon: 'clock',
        title: 'Queued',
        body: "The analysis hasn't started collecting yet.",
        actions: ['cancel'],
      }
    case 'collecting': {
      if (!record.pending_reason) {
        return {
          tone: 'running',
          icon: 'spinner',
          title: 'Reading the session',
          body: 'Capturing the transcript and the session tree. No model is called at this stage.',
          actions: ['cancel'],
        }
      }
      const pending = pendingCopy(record.pending_reason)
      const left = remainingMs(record, now)
      return {
        tone: 'warn',
        icon: 'clock',
        title: pending.title,
        body: `${pending.body} Collection stays pending${left === null ? '' : `; ${formatDuration(left)} left`}.`,
        detail: record.pending_reason,
        actions: ['cancel'],
      }
    }
    case 'judging': {
      const call = record.judge_call
      const seen = signals === 0 ? 'No signals recorded.' : `${plural(signals, 'signal')} recorded.`
      return {
        tone: 'running',
        icon: 'spinner',
        title: 'Jev is classifying the session',
        body: `${seen} ${call ? `The call started at ${clock(call.started_at)}${limits ? ` and times out after ${seconds(limits.judge_timeout_ms, 0)}` : ''}.` : "The call hasn't started yet."}`,
        detail: call ? `request ${call.request_id}` : undefined,
        actions: ['cancel'],
      }
    }
    case 'investigating': {
      const sentence = routingSentence(record, assets.triage)
      const because = sentence?.startsWith('Investigated: ')
        ? `Sent because ${sentence.slice('Investigated: '.length)}`
        : undefined
      return {
        tone: 'running',
        icon: 'spinner',
        title: 'The analyst model is investigating',
        body: `${because ? `${because} ` : ''}${investigationBudget(limits, Boolean(record.code_root))}`,
        detail: `${record.model.model} · ${record.model.provider}`,
        actions: record.analyst ? ['cancel', 'session'] : ['cancel'],
      }
    }
    case 'completed':
      return completedCopy(result)
    case 'failed':
      return failureCopy(result, limits)
    case 'cancelled': {
      const stopped = [...record.stages].reverse().find((stage) => stage.status !== 'cancelled')
      const during = stageWord(stopped?.status)
      const note =
        stopped?.status === 'investigating'
          ? 'The investigation session was stopped; the source session was not touched.'
          : 'The source session was not touched.'
      return {
        tone: 'info',
        icon: 'ban',
        title: `Cancelled during ${during}`,
        body: `It was stopped at ${clock(record.completed_at ?? record.updated_at)}. ${note} Usage so far is kept.`,
        detail: usageSummary(record.usage),
        estimate: true,
        actions: record.analyst ? ['session'] : [],
      }
    }
  }
}
