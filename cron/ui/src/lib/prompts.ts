/** The instructions this page sends to a harness agent. Registration is an
    in-turn intercept — `engine::register_trigger` binds to the session that
    calls it — so a schedule for a conversation can only be created by an
    agent inside that conversation. One copy of the dialect rules lives here
    so the create, replace, and form paths cannot drift apart; every one of
    them was rejected at least once for the same two reasons. */

const ARGUMENT_SHAPE =
  '{"trigger_type":"cron","config":{"expression":"0 30 9 * * Mon"},"label":"<concise label>","once":false}'

const DIALECT = [
  'Arguments, exactly this shape — `config` is a JSON object, never a string:',
  ARGUMENT_SHAPE,
  '',
  'The expression has six fields: second minute hour day-of-month month day-of-week, optionally a seventh year. Five-field Unix expressions are rejected.',
  'Write the day of week as a name (Mon, Tue, Wed, Thu, Fri, Sat, Sun). The numeric form counts Sunday as 1, so a number meant as Monday fires a day early.',
  'Times are UTC.',
]

export function createSchedulePrompt(request: string): string {
  return [
    'Create a scheduled task for this conversation by calling engine::register_trigger exactly once.',
    '',
    ...DIALECT,
    'Omit function_id so each fire wakes this conversation, and keep the label short and faithful to the intent.',
    'Register now and report the subscription_id. Do not only describe the steps.',
    '',
    `Request: ${request}`,
  ].join('\n')
}

export function replaceSchedulePrompt(subscriptionId: string, request: string): string {
  return [
    `Replace scheduled task ${subscriptionId} for this conversation.`,
    'Inspect the existing subscription if the request does not repeat every unchanged detail.',
    '',
    'First register the replacement with engine::register_trigger.',
    ...DIALECT,
    `The replacement must return a subscription_id different from "${subscriptionId}". If it returns the same id, the registration was deduplicated: do not unregister it, and report that the task is unchanged.`,
    `Only after that, call engine::unregister_trigger with {"id":"${subscriptionId}"}.`,
    'If registering the replacement fails, leave the existing subscription registered. Report both operation results. Do not merely explain the steps.',
    '',
    `Requested replacement: ${request}`,
  ].join('\n')
}

export function manualSchedulePrompt(registration: unknown): string {
  return [
    'Create this scheduled task for this conversation.',
    'Call engine::register_trigger exactly once with the JSON object below, verbatim. Do not substitute a timer and do not merely explain the call.',
    'The expression is UTC. Report the returned subscription_id and any registration note.',
    '',
    JSON.stringify(registration, null, 2),
  ].join('\n')
}
