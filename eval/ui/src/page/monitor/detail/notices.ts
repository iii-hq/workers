// The notice that opens an analysis detail in every state but "completed with
// suggestions": what the monitor is doing, or exactly why it stopped. Failures
// never read like a healthy result, and "completed" never speaks for the task.
import { formatBytes, formatDuration } from '@iii-dev/console-ui/format'
import { formatCost, formatTokens, remainingMs, totalTokens } from '../../../model'
import type { AnalysisRecord, AnalysisResult, AnalysisStatus, MonitorLimits, MonitorUsage } from '../../../types'
import {
  clock,
  count,
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
export type NoticeAction = 'cancel' | 'reanalyze' | 'signals' | 'session'

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

function judgeCopy(code: string, limits: MonitorLimits | undefined): { title: string; body: string } {
  switch (code) {
    case 'judge_provider_unavailable':
      return {
        title: 'judge-typesafe is unavailable',
        body: `The provider didn't answer. ${KEPT} No other provider or model was tried.`,
      }
    case 'judge_missing_key':
      return {
        title: 'judge-typesafe has no API key',
        body: `The triage provider has no key configured, so it couldn't classify the session. ${KEPT} Add the key, then reanalyze.`,
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

function failureCopy(result: AnalysisResult, limits: MonitorLimits | undefined): StateCopy | null {
  const { record, assets } = result
  const failure = record.failure
  if (!failure) return null
  const { code, stage } = failure
  const at = record.completed_at ?? record.updated_at
  const signals = record.counters.diagnostics
  const keptSignals =
    signals > 0 ? ` ${plural(signals, 'signal')} from the capture ${signals === 1 ? 'is' : 'are'} kept.` : ''
  const request = record.judge_call?.request_id
  const base = {
    tone: 'alert' as NoticeTone,
    icon: 'alert' as NoticeIcon,
    message: failure.message || undefined,
    detail: [code, clock(at), stage === 'judging' && request ? `request ${request}` : undefined]
      .filter(Boolean)
      .join(' · '),
    actions: ['reanalyze'] as NoticeAction[],
  }
  const retry = 'Reanalyze to try again.'
  const copy = (title: string, body: string, extra: Partial<StateCopy> = {}): StateCopy => ({
    ...base,
    title,
    body,
    ...extra,
  })

  if (code.startsWith('judge_')) {
    const judge = judgeCopy(code, limits)
    return copy(judge.title, judge.body)
  }
  switch (code) {
    case 'deadline':
      return copy(
        'The analysis ran out of time',
        `It didn't finish within ${seconds(record.deadline - record.created_at, 0)}. It was stopped during ${stageWord(stage)}.${assets.snapshot ? ' The evidence captured so far is kept.' : ''} ${retry}`,
        { icon: 'clock' },
      )
    case 'source_not_found':
      return copy(
        'The session has no turn record',
        'Harness has no turn record for this session, so there was nothing to capture. It may have been deleted. No model was called.',
        { icon: 'file-x' },
      )
    case 'source_advanced':
      return copy(
        'The session changed during capture',
        `A newer turn started while turn ${shortId(record.turn_id)} was being read. Turns weren't mixed, so the capture was stopped. Reanalyze once the session settles.`,
      )
    case 'inconsistent_evidence':
      return copy(
        'The session changed during capture',
        "The observed turn resumed, or a descendant session changed, while the evidence was being read. Turns weren't mixed, so the capture was stopped. Reanalyze once the session settles.",
      )
    case 'evidence_unreadable':
      return copy(
        "The evidence couldn't be read",
        `A page of the transcript was missing or malformed, so the monitor stopped instead of skipping evidence. No model was called. ${retry}`,
        { icon: 'file-x' },
      )
    case 'not_a_root_session':
      return copy(
        'This is not a root session',
        "It belongs to another session's tree. Analyze its root session instead: descendants are analyzed together with their root.",
        { tone: 'warn', icon: 'warn', actions: [] },
      )
    case 'monitor_session':
      return copy(
        "The monitor doesn't analyze its own sessions",
        "This session was created by the monitor to investigate another analysis. It's never analyzed, so analyses can't feed each other.",
        { tone: 'warn', icon: 'warn', actions: [] },
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
        forContext ? 'Evidence too large for the analyst' : 'Evidence too large to analyze',
        `${measured} ${forContext ? 'The analyst model was not called.' : 'No model was called.'}${keptSignals} This doesn't mean the session was healthy.`,
        {
          tone: 'warn',
          icon: 'warn',
          message: sizes ? undefined : base.message,
          actions: signals > 0 ? ['signals', 'reanalyze'] : ['reanalyze'],
        },
      )
    }
    case 'snapshot_missing':
      return copy(
        'The saved capture is missing',
        `The evidence captured for this analysis can't be found, so the next stage couldn't run. Reanalyze to capture the session again.`,
        { icon: 'file-x' },
      )
    case 'external_outcome_unknown': {
      const judging = stage !== 'investigating'
      return copy(
        judging ? 'Triage result is unknown' : 'Investigation result is unknown',
        judging
          ? "The eval worker restarted after the Jev call started, and no answer was saved. The call wasn't repeated, so it can't be charged twice. Reanalyze to try again."
          : "The eval worker restarted after the request to the analyst was sent, and no result was saved. The request wasn't repeated, so it can't be charged twice. Reanalyze to try again.",
        { icon: 'help' },
      )
    }
    case 'analyst_rejected':
      return copy(
        "Harness didn't accept the investigation",
        `The request to start the analyst's turn was refused, so there are no suggestions. Triage and signals are kept. ${retry}`,
      )
    case 'analyst_failed':
      return copy(
        'The investigation turn failed',
        `The analyst's turn ended without a result, so there are no suggestions. Triage and signals are kept. ${retry}`,
      )
    case 'analyst_turn_changed':
      return copy(
        'The investigation session changed',
        `Another turn started in the investigation session, so its result can't be trusted. Triage and signals are kept. ${retry}`,
      )
    case 'analyst_step_cap':
      return copy(
        'The investigation ran out of steps',
        `The analyst used every step it was given before it delivered a result, so there are no suggestions. Triage and signals are kept. ${retry}`,
      )
    case 'analyst_output_invalid':
      return copy(
        "The analyst's answer was unusable",
        `The investigation finished, but its result doesn't match the suggestion format, so nothing is shown as a suggestion. Triage and signals are kept. ${retry}`,
      )
    default:
      return copy('The analysis failed', `It stopped during ${stageWord(stage)}. ${retry}`, {
        message: failure.message || undefined,
      })
  }
}

function completedCopy(result: AnalysisResult): StateCopy | null {
  const { record, assets } = result
  if (record.counters.suggestions > 0) return null
  // Every proposal was rejected: the Suggestions section says so.
  if (record.counters.rejected_suggestions > 0) return null
  const signals = record.counters.diagnostics
  const answer = triageChoice(assets.triage)
  const reasons = record.routing?.reasons ?? []
  const parts = [signals === 0 ? 'No signals.' : `${plural(signals, 'signal')} recorded.`]
  if (answer) parts.push(`Triage said ${answer.choice} at ${twoDecimals(answer.confidence)}.`)
  if (record.routing && !record.routing.investigate) parts.push('It was not sent to investigation.')
  else if (record.routing && reasons.length === 1 && reasons[0] === 'audit_sample') {
    parts.push('The session was investigated as part of the audit sample.')
  } else if (record.routing) parts.push('The analyst investigated and proposed no change.')
  // "Nothing worth changing" is only said of a capture that was complete: a
  // missing signal in a partial one is not evidence of healthy behavior.
  const coverage = assets.snapshot?.coverage.level ?? record.coverage
  if (coverage === 'complete') {
    return {
      tone: 'info',
      icon: 'check',
      title: 'Nothing worth changing was found',
      body: parts.join(' '),
      actions: ['reanalyze'],
    }
  }
  if (coverage === 'insufficient' || coverage === 'partial') {
    parts.push(`The capture was ${coverage}, so this doesn't mean the session was healthy.`)
  }
  return {
    tone: coverage === 'insufficient' ? 'warn' : 'info',
    icon: coverage === 'insufficient' ? 'warn' : 'help',
    title: 'No suggestions were made',
    body: parts.join(' '),
    actions: ['reanalyze'],
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
      const sentence = routingSentence(record, assets.triage, limits)
      const because = sentence?.startsWith('Sent to investigation: ')
        ? `Sent because ${sentence.slice('Sent to investigation: '.length)}`
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
        actions: record.analyst ? ['reanalyze', 'session'] : ['reanalyze'],
      }
    }
  }
}
