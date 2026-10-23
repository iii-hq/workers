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
  | 'explore'
  | 'finish'
  | 'closed'

const STEP_TITLES: Record<SetupStepId, string> = {
  welcome: 'Welcome',
  models: 'Models',
  browser: 'Browser',
  judge: 'Judge',
  ready: 'Ready',
  explore: 'Explore iii',
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
 * `onboarding` worker sends them), then what this step adds. A substep
 * (see `finishStep`) may give its own `step_title`; the step's identity
 * stays the step's.
 */
export function setupStepMessage(
  step: SetupStepId,
  stepNumber: number,
  properties: SetupStepProperties,
) {
  return {
    step_title: STEP_TITLES[step],
    ...properties,
    tour_id: SETUP_TOUR_ID,
    tour_title: SETUP_TOUR_TITLE,
    step_id: step,
    step_number: stepNumber,
  }
}

/** What the person confirmed on Explore iii. */
export type FinishChoice =
  | { kind: 'example'; title: string }
  | { kind: 'tour' }
  | { kind: 'own' }
  | { kind: 'finish' }

/**
 * Finish, as the choice made on Explore iii: each choice is a substep of
 * the one `finish` step, with its own `substep_id` and `step_title`. An
 * example is named by its title, read from the project's own file, and
 * identified by that title's slug.
 */
export function finishStep(choice: FinishChoice): SetupStepProperties {
  switch (choice.kind) {
    case 'example':
      return {
        outcome: 'example_prompt',
        substep_id: choice.title
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, '-')
          .replace(/^-|-$/g, ''),
        step_title: choice.title,
      }
    case 'tour':
      return {
        outcome: 'tour',
        substep_id: 'tour',
        step_title: 'Take the learn-iii tour',
      }
    case 'own':
      return {
        outcome: 'own',
        substep_id: 'own',
        step_title: 'Explore on my own',
      }
    case 'finish':
      return { outcome: 'finished', substep_id: 'finish', step_title: 'Finish' }
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
