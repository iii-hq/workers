/**
 * Diagnosis for a turn the provider or iii could not finish.
 *
 * The harness records a failed turn as an `error` custom entry with a stable
 * `code` (`router/provider_auth_expired`, `router/stream_incomplete`, …), a
 * coarse `class` (`llm.transient`, `llm.permanent`, …), the provider's own
 * words in `detail`, and a public `summary`. Those facts are precise about
 * WHAT happened but say nothing about WHO has to act — and that is the one
 * thing a reader needs first: is this my key, my account, a flaky network, or
 * a bug in iii? This module turns the record into that answer.
 *
 * Billing walls are the awkward case: providers report an exhausted balance
 * as a generic permanent rejection (or as a 429 that is not really a rate
 * limit), so only the detail text names the cause. The provider's words
 * therefore win over the transport's class when they are unambiguous.
 */

import type { SystemMessage } from '@/types/chat'

export type TurnFailureCategory =
  | 'auth'
  | 'billing'
  | 'configuration'
  | 'context'
  | 'rate-limit'
  | 'connection'
  | 'rejected'
  | 'send'
  | 'internal'
  | 'unknown'

/**
 * Who has to act. `user`: the provider account, key, or console setup needs a
 * change. `environment`: nobody did anything wrong — networks and provider
 * APIs hiccup, retrying is the fix. `iii`: the engine or the harness failed.
 */
export type TurnFailureOwner = 'user' | 'environment' | 'iii'

export interface TurnFailurePresentation {
  category: TurnFailureCategory
  owner: TurnFailureOwner
  /** Short chip copy naming the owner. */
  ownerLabel: string
  title: string
  /** One or two sentences saying whose problem this is, in plain words. */
  ownership: string
  /** Remediation steps, most useful first. */
  actions: string[]
}

/** Technical-details code the console stamps on a send the engine never accepted. */
export const SEND_FAILED_CODE = 'console.send_failed'
/** The harness kickoff failure prefix the real backend surfaces as a stop reason. */
export const KICKOFF_FAILED_PREFIX = 'harness::send failed'

const BILLING_PATTERN =
  /insufficient[_ ]quota|credit[_ ]balance|credits?\b[^.]{0,40}\b(?:exhausted|too low|depleted|remaining)|exceeded your (?:current )?quota|quota exceeded|\bbilling\b|payment (?:required|method)|hard limit|spending (?:limit|cap)|out of credits|\b402\b|top up|purchase (?:credits|more)|usage limit/i

const AUTH_PATTERN =
  /invalid[_ ]?(?:api[_ ]?)?key|incorrect api key|api[_ ]key (?:is |was )?(?:invalid|missing|not found|revoked|expired)|no api key|authentication|unauthori[sz]ed|permission[_ ]error|permission denied|x-api-key|\b401\b|\b403\b|forbidden|(?:token|credential)s? (?:has |have )?(?:expired|invalid)|not authenticated|auth[_ ]expired/i

const RATE_LIMIT_PATTERN =
  /rate[_ ]?limit|too many requests|\b429\b|throttl|at capacity/i

const CONTEXT_PATTERN =
  /context[_ ](?:window|length|overflow)|too many tokens|maximum context|(?:prompt|input) is too long|\b413\b|token limit/i

const CONNECTION_PATTERN =
  /without a terminal frame|connection (?:reset|refused|closed|lost|error|aborted)|\becon\w+|etimedout|timed? ?out|socket hang up|network|unreachable|disconnect|\beof\b|broken pipe|overloaded|\b50[234]\b|\b529\b|bad gateway|service unavailable|upstream|stream (?:ended|closed|incomplete|idle|setup)/i

const CONFIGURATION_CODES = new Set([
  'router/not_configured',
  'router/unknown_provider',
  'router/no_provider_for_model',
  'router/ambiguous_model',
  'router/structured_output_unsupported',
  'router/invalid_request',
])

const CONNECTION_CODES = new Set([
  'router/stream_setup_failed',
  'router/stream_idle_timeout',
  'router/stream_incomplete',
  'router/provider_unavailable',
  'router/provider_transient',
])

const RATE_LIMIT_CODES = new Set([
  'router/provider_rate_limited',
  'router/capacity_exceeded',
])

const INTERNAL_CODES = new Set(['router/request_in_progress'])

export function categorizeTurnFailure(input: {
  code?: string
  class?: string
  text: string
}): TurnFailureCategory {
  const code = input.code ?? ''
  const klass = input.class ?? ''
  const text = input.text

  if (code === SEND_FAILED_CODE) return 'send'
  if (
    code.startsWith('harness.') ||
    INTERNAL_CODES.has(code) ||
    text.includes(KICKOFF_FAILED_PREFIX)
  ) {
    return 'internal'
  }
  // The provider's own words win over the transport's coarse class: a billing
  // wall arrives as a generic permanent rejection (or a 429 that is really a
  // quota), and only the detail names the cause.
  if (BILLING_PATTERN.test(text)) return 'billing'
  if (
    klass === 'llm.auth_expired' ||
    code === 'router/provider_auth_expired' ||
    AUTH_PATTERN.test(text)
  ) {
    return 'auth'
  }
  if (CONFIGURATION_CODES.has(code)) return 'configuration'
  if (code === 'router/context_overflow') return 'context'
  if (RATE_LIMIT_CODES.has(code)) return 'rate-limit'
  if (CONNECTION_CODES.has(code)) return 'connection'
  if (code === 'router/provider_rejected') return 'rejected'
  switch (klass) {
    case 'llm.context_overflow':
      return 'context'
    case 'llm.rate_limited':
      return 'rate-limit'
    case 'llm.transient':
      return 'connection'
    case 'llm.permanent':
      return 'rejected'
  }
  if (CONTEXT_PATTERN.test(text)) return 'context'
  if (RATE_LIMIT_PATTERN.test(text)) return 'rate-limit'
  if (CONNECTION_PATTERN.test(text)) return 'connection'
  return 'unknown'
}

const OWNER_BY_CATEGORY: Record<TurnFailureCategory, TurnFailureOwner> = {
  auth: 'user',
  billing: 'user',
  configuration: 'user',
  context: 'user',
  rejected: 'user',
  'rate-limit': 'environment',
  connection: 'environment',
  send: 'environment',
  internal: 'iii',
  unknown: 'environment',
}

const OWNER_LABEL: Record<TurnFailureOwner, string> = {
  user: 'Needs your attention',
  environment: 'Can happen · retry',
  iii: 'iii error',
}

function titleFor(category: TurnFailureCategory, code: string): string {
  switch (category) {
    case 'auth':
      return 'Provider credentials rejected'
    case 'billing':
      return 'Provider credit or quota exhausted'
    case 'configuration':
      switch (code) {
        case 'router/unknown_provider':
          return 'Provider no longer registered'
        case 'router/no_provider_for_model':
          return 'No provider serves this model'
        case 'router/ambiguous_model':
          return 'Model offered by several providers'
        case 'router/structured_output_unsupported':
          return 'Structured output not supported'
        case 'router/invalid_request':
          return 'Request rejected as invalid'
        default:
          return 'Provider not configured'
      }
    case 'context':
      return 'Conversation too large for this model'
    case 'rate-limit':
      return 'Provider is busy right now'
    case 'connection':
      switch (code) {
        case 'router/stream_idle_timeout':
          return 'Provider stopped responding'
        case 'router/provider_unavailable':
          return 'Provider temporarily unavailable'
        case 'router/stream_setup_failed':
          return 'Response stream could not start'
        default:
          return 'Connection to the provider dropped'
      }
    case 'rejected':
      return 'Provider rejected the request'
    case 'send':
      return 'Message could not be sent'
    case 'internal':
      return 'iii could not complete the turn'
    case 'unknown':
      return 'Response could not be completed'
  }
}

function ownershipFor(
  category: TurnFailureCategory,
  provider: string | undefined,
): string {
  const api = provider ? `The ${provider} API` : 'The provider'
  switch (category) {
    case 'auth':
      return `The credentials configured for ${provider ?? 'this provider'} were refused. This is a problem with the API key or account on the provider side, not with iii or the console.`
    case 'billing':
      return `The ${provider ?? 'provider'} account has run out of credit or quota. This is a billing limit on the provider side, not an iii or console failure.`
    case 'configuration':
      return 'No provider is set up to serve this request. This is a console setup gap on your side, not a provider outage or an iii bug.'
    case 'context':
      return 'The conversation has outgrown the context window of the selected model. Nothing is broken; the history is simply too large to send.'
    case 'rate-limit':
      return `${api} is throttling requests right now. This happens under load and is not caused by you or by iii.`
    case 'connection':
      return `The connection to ${provider ?? 'the provider'} dropped before the response finished. This can happen with provider APIs and networks, and is usually not caused by anything you did.`
    case 'rejected':
      return `${api} rejected this request. Something in the selected model or its provider settings is not accepted on the provider side.`
    case 'send':
      return 'The console lost contact with iii while sending. This is a connection problem between the console and the iii engine, not a problem with your message.'
    case 'internal':
      return 'iii could not run this turn. This looks like a problem inside iii rather than with your provider or your request.'
    case 'unknown':
      return 'The turn ended before a response could be completed. The technical details below carry what the provider and iii reported.'
  }
}

function defaultActionsFor(
  category: TurnFailureCategory,
  provider: string | undefined,
): string[] {
  switch (category) {
    case 'auth':
      return [
        `Check the API key or credentials for ${provider ?? 'the provider'} in the model picker (Configure provider).`,
        'Retry the turn once the credentials are updated.',
      ]
    case 'billing':
      return [
        `Add credit or raise the usage limit in the ${provider ?? 'provider'} account.`,
        'Or pick a model from another provider and retry.',
      ]
    case 'configuration':
      return [
        'Open the model picker and finish the provider setup.',
        'Retry once the provider is configured.',
      ]
    case 'context':
      return [
        'Send /compact to summarise the conversation so far.',
        'Or switch to a model with a larger context window.',
      ]
    case 'rate-limit':
      return [
        'Wait a moment, then retry the turn.',
        'Or pick another provider or model.',
      ]
    case 'connection':
      return [
        'Retry the turn; any partial output above is preserved.',
        'If it keeps happening, check the provider status page and your network.',
      ]
    case 'rejected':
      return [
        'Review the selected model and its provider settings, then retry.',
        "The technical details below carry the provider's own message.",
      ]
    case 'send':
      return [
        'Check that the iii engine and the harness worker are running.',
        'Retry sending the message.',
      ]
    case 'internal':
      return [
        'Retry the turn.',
        'If it happens again, check the harness worker logs; the technical details identify the turn.',
      ]
    case 'unknown':
      return [
        'Inspect the technical details below.',
        'Retry after correcting the request or the provider setup.',
      ]
  }
}

/**
 * Categories where the console's own steps are more specific than the
 * harness's generic "review the provider settings" advice.
 */
const PREFER_CONSOLE_ACTIONS = new Set<TurnFailureCategory>([
  'auth',
  'billing',
  'send',
])

export function classifyTurnFailure(
  message: Pick<SystemMessage, 'content' | 'technicalDetails' | 'failure'> & {
    nextActions?: string[]
  },
): TurnFailurePresentation {
  const details = message.technicalDetails
  const code = details?.code ?? ''
  const text = [details?.detail, message.failure?.summary, message.content]
    .filter((part): part is string => typeof part === 'string')
    .join('\n')
  const category = categorizeTurnFailure({
    code,
    class: details?.class,
    text,
  })
  const owner = OWNER_BY_CATEGORY[category]
  const provider = details?.provider
  const harnessActions = message.nextActions?.filter(
    (action) => action.trim().length > 0,
  )
  const actions =
    !PREFER_CONSOLE_ACTIONS.has(category) && harnessActions?.length
      ? harnessActions
      : defaultActionsFor(category, provider)
  return {
    category,
    owner,
    ownerLabel: OWNER_LABEL[owner],
    title: titleFor(category, code),
    ownership: ownershipFor(category, provider),
    actions,
  }
}
