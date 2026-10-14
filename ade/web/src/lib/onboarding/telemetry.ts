import { getIiiClient } from '@/lib/iii-client'

/**
 * Setup wizard usage, reported the way the guided tour reports its own: one
 * durable publish on `onboarding:steps:complete` per closed step, which the
 * engine's `telemetry` worker subscribes to and forwards as one
 * `onboarding_step` product event carrying this payload (see the engine's
 * `workers/telemetry/onboarding.rs`). The browser never talks to an
 * analytics vendor: it announces, and an engine the operator opted out of
 * telemetry never listens.
 *
 * The engine reports each `(tour_id, step_id)` once per run, so a step left
 * twice (Back, then forward again) counts once.
 *
 * Nothing a person typed is reported: no keys, prompts, paths or error
 * text — step ids, outcomes, provider and judge ids, counts and durations.
 */
export const STEP_TOPIC = 'onboarding:steps:complete'
export const SETUP_TOUR_ID = 'setup'
const SETUP_TOUR_TITLE = 'Set up the harness'
const PUBLISH_TIMEOUT_MS = 10_000

export type SetupStepId =
  | 'welcome'
  | 'models'
  | 'browser'
  | 'judge'
  | 'ready'
  | 'finish'
  | 'closed'

const STEP_TITLES: Record<SetupStepId, string> = {
  welcome: 'Welcome',
  models: 'Models',
  browser: 'Browser',
  judge: 'Judge',
  ready: 'Ready',
  finish: 'Finish',
  closed: 'Closed setup',
}

/** What the event carries beyond the step itself. */
export type SetupStepProperties = Record<
  string,
  string | number | boolean | null | readonly string[]
>

/**
 * The message published for one closed step: the tour's own five fields
 * (`tour_id`, `tour_title`, `step_number`, `step_id`, `step_title`, as the
 * `onboarding` worker sends them), then what this step adds.
 */
export function setupStepMessage(
  step: SetupStepId,
  stepNumber: number,
  properties: SetupStepProperties,
) {
  return {
    ...properties,
    tour_id: SETUP_TOUR_ID,
    tour_title: SETUP_TOUR_TITLE,
    step_id: step,
    step_title: STEP_TITLES[step],
    step_number: stepNumber,
  }
}

/** Announce a closed step. Best effort: setup never waits on it or fails. */
export function reportSetupStep(
  step: SetupStepId,
  stepNumber: number,
  properties: SetupStepProperties = {},
): void {
  void getIiiClient()
    .then((client) =>
      client.trigger(
        'iii::durable::publish',
        {
          topic: STEP_TOPIC,
          data: setupStepMessage(step, stepNumber, properties),
        },
        { timeoutMs: PUBLISH_TIMEOUT_MS },
      ),
    )
    .catch(() => undefined)
}
