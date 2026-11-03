/** The instructions this page sends to a harness agent. Observing is an
    in-turn intercept — `harness::observe` creates the observer for the session
    that calls it — so a schedule for a conversation can only be created by an
    agent inside that conversation. One copy of the dialect rules lives here
    so the create, replace, and form paths cannot drift apart; every one of
    them was rejected at least once for the same two reasons. */

const ARGUMENT_SHAPE =
  '{"function_id":"cron::on-tick","args":{"expression":"0 30 9 * * Mon"},"label":"<concise label>","once":false}'

const DIALECT = [
  'Arguments, exactly this shape — `args` is a JSON object, never a string:',
  ARGUMENT_SHAPE,
  '',
  'The expression has six fields: second minute hour day-of-month month day-of-week, optionally a seventh year. Five-field Unix expressions are rejected.',
  'Write the day of week as a name (Mon, Tue, Wed, Thu, Fri, Sat, Sun). The numeric form counts Sunday as 1, so a number meant as Monday fires a day early.',
  'Times are UTC.',
]

export function createSchedulePrompt(request: string): string {
  return [
    'Create a scheduled task for this conversation by calling harness::observe exactly once.',
    '',
    ...DIALECT,
    'Omit target so each fire wakes this conversation, and keep the label short and faithful to the intent.',
    'Observe now and report the observer_id. Do not only describe the steps.',
    '',
    `Request: ${request}`,
  ].join('\n')
}

export function replaceSchedulePrompt(observerId: string, request: string): string {
  return [
    `Replace scheduled task ${observerId} for this conversation.`,
    'Inspect the existing observer if the request does not repeat every unchanged detail.',
    '',
    'First create the replacement with harness::observe.',
    ...DIALECT,
    `The replacement must return an observer_id different from "${observerId}". If it returns the same id, the request was deduplicated: do not unobserve it, and report that the task is unchanged.`,
    `Only after that, call harness::unobserve with {"id":"${observerId}"}.`,
    'If creating the replacement fails, leave the existing observer in place. Report both operation results. Do not merely explain the steps.',
    '',
    `Requested replacement: ${request}`,
  ].join('\n')
}

export function manualSchedulePrompt(observation: unknown): string {
  return [
    'Create this scheduled task for this conversation.',
    'Call harness::observe exactly once with the JSON object below, verbatim. Do not substitute a timer and do not merely explain the call.',
    'The expression is UTC. Report the returned observer_id and any note.',
    '',
    JSON.stringify(observation, null, 2),
  ].join('\n')
}
