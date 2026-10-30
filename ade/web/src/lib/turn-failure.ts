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
  /**
   * Provider-declared guidance for a context overflow (`context_overflow_hint`
   * on `router::provider::list`), shown under the steps. Only set for the
   * `context` category.
   */
  providerNote?: string
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
  // Assembly overflows are context failures, including older llm.permanent records.
  if (code === 'harness.context_overflow') return 'context'
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
  user: 'Action needed',
  environment: 'Temporary',
  iii: 'iii issue',
}

/**
 * The connection never opened: DNS, refused, no route. On a laptop that is
 * nearly always the user's network (offline, VPN, captive portal), not the
 * provider, so the copy points there first.
 */
const UNREACHABLE_PATTERN =
  /dns error|failed to lookup|nodename nor servname|name or service not known|getaddrinfo|enotfound|eai_again|connection refused|econnrefused|network is unreachable|no route to host|client error \(connect\)/i

/** Every sentence below names the provider the way people do: `OpenAI`. */
interface Copy {
  title: string
  ownership: string
  actions: string[]
}

function copyFor(
  category: TurnFailureCategory,
  code: string,
  provider: string | undefined,
  unreachable: boolean,
): Copy {
  const name = provider ?? 'The provider'
  const lower = provider ?? 'the provider'
  switch (category) {
    case 'auth':
      return {
        title: `${name} didn’t accept your API key`,
        ownership: `${name} says the API key is wrong, expired or missing. Nothing is broken in iii; the key needs updating.`,
        actions: [
          `Update the API key for ${lower} in the model picker (Configure provider).`,
          'Then try again.',
        ],
      }
    case 'billing':
      return {
        title: `Your ${provider ?? 'provider'} account is out of credit`,
        ownership: `${name} stopped the request because the account has no credit left or hit its usage limit. This is a billing limit on the provider’s side, not an iii or ADE failure.`,
        actions: [
          `Add credit or raise the usage limit in your ${provider ?? 'provider'} account.`,
          'Or switch to a model from another provider.',
        ],
      }
    case 'configuration': {
      const title =
        code === 'router/unknown_provider'
          ? `${name} isn’t set up anymore`
          : code === 'router/no_provider_for_model'
            ? 'No provider is set up for this model'
            : code === 'router/ambiguous_model'
              ? 'More than one provider offers this model'
              : code === 'router/structured_output_unsupported'
                ? 'This model can’t return structured output'
                : code === 'router/invalid_request'
                  ? `${name} rejected the request as invalid`
                  : `${name} isn’t set up yet`
      return {
        title,
        ownership:
          'The model you picked doesn’t have a working provider behind it yet.',
        actions: [
          'Open the model picker and finish setting up the provider.',
          'Then try again.',
        ],
      }
    }
    case 'context':
      return {
        title: 'This conversation is too long for the model',
        ownership:
          'The conversation has grown past what the selected model can read at once, and it couldn’t be shortened enough to send.',
        actions: [
          'Send /compact to shorten the conversation.',
          'If that fails, switch to a model with a larger context window, compact there, then switch back before switching again.',
          'Or start a new conversation.',
        ],
      }
    case 'rate-limit':
      return {
        title: `${name} is busy right now`,
        ownership: `${name} is getting too many requests. This usually clears up within a minute and isn’t caused by you.`,
        actions: [
          'Wait a minute, then try again.',
          'Or switch to another model or provider.',
        ],
      }
    case 'connection':
      if (unreachable) {
        return {
          title: `Couldn’t reach ${name}`,
          ownership: `Your computer couldn’t connect to ${lower}. This is usually the internet connection, a VPN or a firewall, not ${lower} itself.`,
          actions: [
            'Check that you’re online and any VPN or proxy is working.',
            'Then try again; anything already written above is kept.',
          ],
        }
      }
      return {
        title:
          code === 'router/stream_idle_timeout'
            ? `${name} stopped responding`
            : code === 'router/provider_unavailable'
              ? `${name} is down right now`
              : code === 'router/stream_setup_failed'
                ? `Couldn’t start a reply from ${lower}`
                : `Lost the connection to ${lower}`,
        ownership: `The connection to ${lower} broke before the reply finished. It’s usually a short network or provider hiccup, not caused by anything you did.`,
        actions: [
          'Try again; anything already written above is kept.',
          `If it keeps happening, check ${lower}’s status page.`,
        ],
      }
    case 'rejected':
      return {
        title: `${name} rejected the request`,
        ownership: `${name} refused this request. The selected model or one of its settings may not be supported there.`,
        actions: [
          'Check the model and its provider settings, then try again.',
          'Technical details has the provider’s exact message.',
        ],
      }
    case 'send':
      return {
        title: 'Your message didn’t send',
        ownership:
          'The app lost its connection to iii while sending. Nothing is wrong with your message.',
        actions: [
          'Make sure iii and the harness worker are running.',
          'Then send the message again.',
        ],
      }
    case 'internal':
      return {
        title: 'Something went wrong in iii',
        ownership:
          'iii hit an error while running this turn. It isn’t caused by your provider or your message.',
        actions: [
          'Try again.',
          'If it happens again, check the harness worker logs.',
        ],
      }
    case 'unknown':
      return {
        title: 'The reply didn’t finish',
        ownership:
          'The reply stopped before it was complete. Technical details shows what was reported.',
        actions: [
          'Look at Technical details for the cause.',
          'Fix the request or the provider setup, then try again.',
        ],
      }
  }
}

/**
 * Categories where the console's own steps are more specific than the
 * harness's generic "review the provider settings" advice.
 */
const PREFER_CONSOLE_ACTIONS = new Set<TurnFailureCategory>([
  'auth',
  'billing',
  'context',
  'send',
  'rate-limit',
  'connection',
  'internal',
  'unknown',
])

export function classifyTurnFailure(
  message: Pick<SystemMessage, 'content' | 'technicalDetails' | 'failure'> & {
    nextActions?: string[]
  },
  options: {
    providerHint?: string
    /** How people name the provider (`OpenAI`); defaults to its id. */
    providerName?: string
  } = {},
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
  const provider = options.providerName ?? details?.provider
  const copy = copyFor(category, code, provider, UNREACHABLE_PATTERN.test(text))
  const harnessActions = message.nextActions?.filter(
    (action) => action.trim().length > 0,
  )
  // The harness's steps are generic ("Retry the turn in a moment"); ours are
  // plainer wherever we know the case.
  const actions =
    !PREFER_CONSOLE_ACTIONS.has(category) && harnessActions?.length
      ? harnessActions
      : copy.actions
  // Provider-declared add-on: only meaningful under a context overflow.
  const providerNote =
    category === 'context'
      ? options.providerHint?.trim() || undefined
      : undefined
  return {
    category,
    owner,
    ownerLabel: OWNER_LABEL[owner],
    title: copy.title,
    ownership: copy.ownership,
    actions,
    ...(providerNote ? { providerNote } : {}),
  }
}
