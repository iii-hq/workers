/**
 * learn-iii — the guided console tour.
 *
 * Split of responsibility:
 *   - this worker owns CONTENT (src/tours.mjs) and PROGRESS (kept in the
 *     `state` worker, like any other iii application keeps its data)
 *   - the injected page (ui/page.tsx) owns RENDERING: the step list and the
 *     spotlight it draws over the console
 *
 * The page reaches this worker through the console's normal function calls,
 * so there is no private console bridge to keep in step.
 */

import { existsSync, watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerWorker } from 'iii-sdk'
import {
  DEFAULT_SIGNUP_URL,
  errorCode,
  isEmailish,
  isSignupUrlAllowed,
  SIGNUP_SOURCE,
  signupOutcome,
} from './subscribe.mjs'
import { findStep, getTour, listTours, TOURS } from './tours.mjs'

const WORKER = 'onboarding'
const STATE_SCOPE = 'learn-iii'
const STATE_TIMEOUT_MS = 10_000

const iii = registerWorker(process.env.III_URL ?? process.env.III_ENGINE_URL, {
  workerName: WORKER,
  workerDescription:
    'Guided console tour — step content, per-operator progress, and the injected page that spotlights the console.',
})

const object = (properties = {}, required = []) => ({
  type: 'object',
  properties,
  ...(required.length ? { required } : {}),
})
const string = { type: 'string' }
const integer = { type: 'integer' }

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

iii.registerFunction('learn-iii::tours::list', async () => ({ tours: listTours() }), {
  description:
    'List every available tour in curriculum order with its id, title, description, and step count. Call this to find out what learn-iii content exists.',
  request_format: object(),
  response_format: object({ tours: { type: 'array' } }, ['tours']),
})

iii.registerFunction(
  'learn-iii::tours::get',
  async (input) => {
    const tour = getTour(input.id)
    if (!tour) throw new Error(`unknown tour: ${input.id}`)
    return { tour }
  },
  {
    description:
      'Fetch one tour by id with every step: id, title, body, and the CSS anchor the console page spotlights for that step.',
    request_format: object({ id: string }, ['id']),
    response_format: object({ tour: object() }, ['tour']),
  },
)

// ---------------------------------------------------------------------------
// Progress — one state key per subject, so a reload resumes mid-tour
// ---------------------------------------------------------------------------

const progressKey = (subject) => `progress:${subject}`

const stateGet = (key) =>
  iii.call({
    function_id: 'state::get',
    payload: { scope: STATE_SCOPE, key },
    timeoutMs: STATE_TIMEOUT_MS,
  })

/**
 * Progress is written with ordered atomic ops, never read-then-replace: two
 * handlers can interleave around an await, and the second `state::set` would
 * carry a snapshot that predates the first one's write. `state::update`
 * touches only the paths named here.
 *
 * `merge` inserts keys at the path it walks (creating what is missing), so a
 * step lands under its own tour without the rest of the value passing
 * through this worker at all.
 */
const stateUpdate = (key, ops) =>
  iii.call({
    function_id: 'state::update',
    payload: { scope: STATE_SCOPE, key, ops },
    timeoutMs: STATE_TIMEOUT_MS,
  })

/** The shape the page reads, whether or not anything is stored yet. */
const emptyProgress = () => ({ tours: {}, updated_at: null, subscribed_at: null })

const readProgress = async (subject) => {
  const stored = await stateGet(progressKey(subject))
  const progress = stored && typeof stored === 'object' ? stored : emptyProgress()
  // A reset nulls its tour rather than removing it — `state::update`'s
  // `remove` reaches top-level keys only — so nulls are dropped on the way
  // out and the page never sees a tour it cannot read.
  const tours = Object.fromEntries(
    Object.entries(progress.tours ?? {}).filter(([, record]) => record && typeof record === 'object'),
  )
  return { ...emptyProgress(), ...progress, tours }
}

const tourIsComplete = (tour, record) =>
  tour.steps.every((step) => record?.steps?.[step.id]?.status === 'complete')

/**
 * A fired condition is evidence the page shows back to the operator, so it is
 * stored with the step. ponytail: payloads are capped rather than paged —
 * `state` refuses oversized values, and a tour step needs the shape of the
 * event, not every byte of it.
 */
const FIRED_PAYLOAD_LIMIT = 4_000

const cap = (payload) => {
  if (payload === undefined) return null
  const json = JSON.stringify(payload) ?? 'null'
  if (json.length <= FIRED_PAYLOAD_LIMIT) return payload
  return { truncated: true, bytes: json.length, head: json.slice(0, FIRED_PAYLOAD_LIMIT) }
}

iii.registerFunction(
  'learn-iii::progress::get',
  async (input) => {
    const progress = await readProgress(input?.subject ?? 'local')
    const next = listTours().find((entry) => {
      const tour = getTour(entry.id)
      return tour && !tourIsComplete(tour, progress.tours[entry.id])
    })
    return { ...progress, next_tour_id: next?.id ?? null }
  },
  {
    description:
      'Read an operator\u2019s tour progress: per tour, the status of every step and the observed event that closed it, plus which tour to offer next.',
    request_format: object({ subject: string }),
    response_format: object(
      {
        tours: object(),
        next_tour_id: { type: ['string', 'null'] },
        subscribed_at: { type: ['number', 'null'] },
      },
      ['tours', 'next_tour_id'],
    ),
  },
)

/**
 * The topic every completed step is announced on, so anything that wants to
 * follow a tour (the engine's telemetry among them) subscribes to the topic
 * and never reads this worker's state.
 */
const STEP_TOPIC = 'onboarding:steps:complete'

/**
 * Published through the `queue` worker, because a step closes once: the
 * message waits in each subscriber's durable queue until that subscriber
 * takes it, and a failed delivery is retried.
 *
 * A subscriber observes `queue::on-message` with args `{ topic: STEP_TOPIC }`
 * under a stable observer id (`{ id: '<subscriber>' }`). The queue is keyed
 * by that id, so a subscriber that restarts under the same worker name and id
 * within the engine's grace period takes its queue back, with the steps
 * published while it was away. The engine's telemetry subscribes as
 * `iii-telemetry::onboarding-step-watch`.
 *
 * The publish itself is best effort: a missing `queue` worker is logged and
 * never fails the step.
 */
const publishStep = (data) =>
  iii
    .call({
      function_id: 'iii::durable::publish',
      payload: { topic: STEP_TOPIC, data },
      timeoutMs: STATE_TIMEOUT_MS,
    })
    .catch((cause) => {
      console.error(`[${WORKER}] could not publish ${STEP_TOPIC}: ${cause?.message ?? cause}`)
    })

/** Close one step for one subject; the body of `learn-iii::steps::complete`. */
async function completeStep(input) {
  const found = findStep(input.tour_id, input.step_id)
  if (!found) throw new Error(`unknown step: ${input.tour_id}/${input.step_id}`)
  const { tour, step } = found
  const subject = input.subject ?? 'local'
  // A step closes once per subject, so a reload or a second report from the
  // agent must not announce it again.
  // ponytail: read-then-publish, not atomic — two reports racing around this
  // await can both announce. Move the check into `state::update` if a
  // duplicate ever matters more than the round trip does.
  const announced =
    (await readProgress(subject)).tours?.[input.tour_id]?.steps?.[input.step_id]?.status ===
    'complete'
  const at = Date.now()
  const record = {
    status: 'complete',
    at,
    // Present only when an observed event closed the step, so the page can
    // tell a condition apart from a step the operator closed by hand.
    fired: input.fired
      ? {
          function_id: input.fired.function_id ?? step.condition?.function_id ?? null,
          payload: cap(input.fired.payload),
          at,
        }
      : null,
  }
  await stateUpdate(progressKey(subject), [
    { type: 'merge', path: ['tours', input.tour_id, 'steps'], value: { [input.step_id]: record } },
    { type: 'merge', path: ['tours', input.tour_id], value: { updated_at: at } },
    { type: 'merge', value: { updated_at: at } },
  ])
  if (!announced) {
    await publishStep({
      tour_id: input.tour_id,
      tour_title: tour.title,
      step_number: found.number,
      step_id: input.step_id,
      step_title: step.title,
    })
  }
  return { step: record }
}

iii.registerFunction(
  'learn-iii::steps::complete',
  completeStep,
  {
    description:
      'Mark one tour step complete. Pass `fired` when an observed event closed it \u2014 the observed function id and the event are kept as the evidence the tour shows back.',
    request_format: object(
      {
        subject: string,
        tour_id: string,
        step_id: string,
        fired: object({
          function_id: string,
          payload: {},
        }),
      },
      ['tour_id', 'step_id'],
    ),
    response_format: object({ step: object() }, ['step']),
  },
)

iii.registerFunction(
  'learn-iii::steps::reset',
  async (input) => {
    // Without this an unknown id reports success and leaves its own null key
    // under `tours` forever.
    if (!getTour(input.tour_id)) throw new Error(`unknown tour: ${input.tour_id}`)
    const subject = input.subject ?? 'local'
    const at = Date.now()
    await stateUpdate(progressKey(subject), [
      // `remove` reaches top-level keys only, so the tour is nulled instead;
      // `readProgress` drops nulls on the way out.
      { type: 'merge', path: ['tours'], value: { [input.tour_id]: null } },
      { type: 'merge', value: { updated_at: at } },
    ])
    return { reset: true }
  },
  {
    description: 'Forget every step of one tour, so it starts from the beginning again.',
    request_format: object({ subject: string, tour_id: string }, ['tour_id']),
    response_format: object({ reset: { type: 'boolean' } }, ['reset']),
  },
)

// ---------------------------------------------------------------------------
// Product updates
// ---------------------------------------------------------------------------

const SIGNUP_TIMEOUT_MS = 10_000

/**
 * The POST happens here and not in the browser: the injected page has no
 * network of its own, and one place to change the destination is enough.
 */
const SIGNUP_URL = process.env.III_LEARN_III_SIGNUP_URL ?? DEFAULT_SIGNUP_URL

// The operator's address goes over this URL, so an override that downgrades it
// to plain HTTP anywhere but this machine is refused rather than used.
if (!isSignupUrlAllowed(SIGNUP_URL)) {
  throw new Error(`III_LEARN_III_SIGNUP_URL must be an https URL, or http to this machine: ${SIGNUP_URL}`)
}

/**
 * The topic a captured address is announced on. The engine's telemetry worker
 * subscribes and writes it to the person keyed by that machine's `device_id`,
 * which only the engine knows.
 */
const IDENTIFY_TOPIC = 'email:signup'

/**
 * Best effort, and after the list has taken the address: an engine with
 * telemetry off drains this topic, and a project without the `queue` worker
 * never delivers it. Neither outcome may fail the signup the operator asked
 * for.
 */
const publishIdentify = (email, source) =>
  iii
    .call({
      function_id: 'iii::durable::publish',
      payload: { topic: IDENTIFY_TOPIC, data: { email, source } },
      timeoutMs: STATE_TIMEOUT_MS,
    })
    .catch((cause) => {
      // The address itself is never logged.
      console.error(`[${WORKER}] could not publish ${IDENTIFY_TOPIC}: ${cause?.message ?? cause}`)
    })

/**
 * POST the address to the API and say what its answer means. Never throws:
 * an API that cannot be reached is a `retry_later` like any other outage.
 */
const postSignup = async (email) => {
  let response
  try {
    // A signup host that accepts the connection and never answers would hold
    // this invocation open until the caller gives up, with the socket still
    // in hand. The deadline is ours, not the caller's.
    response = await fetch(SIGNUP_URL, {
      method: 'POST',
      // No Origin header: the API accepts server-to-server calls without one.
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ email, source: SIGNUP_SOURCE }),
      // A redirect would carry the address to a host we never checked, so a
      // moved endpoint is an error here rather than a silent second request.
      redirect: 'error',
      signal: AbortSignal.timeout(SIGNUP_TIMEOUT_MS),
    })
  } catch (cause) {
    // fetch's message names the URL at most, never the body.
    console.error(`[${WORKER}] the signup API could not be reached: ${cause?.name ?? ''} ${cause?.message ?? cause}`)
    return 'retry_later'
  }
  const error = errorCode(await response.text().catch(() => ''))
  // One line per attempt, so a signup can be followed in the worker log. The
  // address itself is never logged.
  console.log(`[${WORKER}] signup API answered ${response.status}${error ? ` ${error}` : ''}`)
  return signupOutcome(response.status, error)
}

iii.registerFunction(
  'learn-iii::subscribe',
  async (input) => {
    const email = String(input.email ?? '').trim()
    if (!isEmailish(email)) return { subscribed: false, outcome: 'invalid_email' }
    const outcome = await postSignup(email)
    if (outcome !== 'subscribed') return { subscribed: false, outcome }
    await publishIdentify(email, String(input.source ?? 'learn-iii'))
    // Remembered beside the tour progress (the time only, never the address),
    // so a reopened Stay in touch step says the person signed up instead of
    // asking again.
    const at = Date.now()
    await stateUpdate(progressKey(input?.subject ?? 'local'), [
      { type: 'merge', value: { subscribed_at: at, updated_at: at } },
    ])
    return { subscribed: true, outcome }
  },
  {
    description:
      'Add an email address to the iii product-update list (iii sends a confirmation email first), and record the signup time (not the address) on the subject\u2019s progress as `subscribed_at`. `outcome` is `subscribed`, `invalid_email` (show it), or `retry_later` (the API could not take it now; ask another time).',
    request_format: object({ email: string, source: string, subject: string }, ['email']),
    response_format: object(
      {
        subscribed: { type: 'boolean' },
        outcome: { type: 'string', enum: ['subscribed', 'invalid_email', 'retry_later'] },
      },
      ['subscribed', 'outcome'],
    ),
  },
)

// ---------------------------------------------------------------------------
// Injected console UI
// ---------------------------------------------------------------------------

const uiAssets = {
  'learn-iii/page.js': {
    file: 'page.js',
    kind: 'script',
    content_type: 'text/javascript',
  },
  'learn-iii/styles.css': {
    file: 'styles.css',
    kind: 'style',
    content_type: 'text/css',
  },
}

/**
 * Assets are read from disk at call time rather than inlined at build time, so
 * the watcher below is the same code path as production.
 *
 * Two layouts hold them: beside the published `dist/bundle/index.mjs`, and in
 * `ui/dist` in a source checkout. The first one that has the page wins.
 */
const here = dirname(fileURLToPath(import.meta.url))
const uiDir =
  process.env.III_LEARN_III_UI_DIR ??
  [here, join(here, '..', 'ui', 'dist')].find((candidate) => existsSync(join(candidate, 'page.js'))) ??
  join(here, '..', 'ui', 'dist')

iii.registerFunction(
  'learn-iii::ui-content',
  async (input) => {
    const asset = uiAssets[input.path]
    if (!asset) throw new Error(`unknown ui asset: ${input.path}`)
    const content = await readFile(join(uiDir, asset.file), 'utf8')
    return { content, content_type: asset.content_type }
  },
  {
    description: 'Serve the injectable learn-iii console page assets.',
    metadata: { internal: true },
    request_format: object({ path: string }, ['path']),
    response_format: object({ content: string, content_type: string }, [
      'content',
      'content_type',
    ]),
  },
)

// One `ade::ui::asset` observer per asset; the console fetches its
// content by calling through the observer with `{ path }`.
const observeUiAsset = (path) => {
  const handle = iii.observe(
    { function_id: 'ade::ui::asset', args: { kind: uiAssets[path].kind, path } },
    'learn-iii::ui-content',
  )
  handle.onStateChange((state, reason) => {
    // Hot reload observes the path afresh, which supersedes the previous one.
    if (state === 'error' && reason?.code !== 'UI_ASSET_SUPERSEDED') {
      console.error(`[${WORKER}] ui asset ${path} rejected: ${reason?.code} ${reason?.message ?? ''}`)
    }
  })
  return handle
}

const uiObservers = new Map(
  Object.keys(uiAssets).map((path) => [path, observeUiAsset(path)]),
)

// Observing a path afresh with different bytes hot-swaps the asset in every
// open console tab, so editing the page is one rebuild away from being visible.
if (process.env.III_LEARN_III_UI_WATCH) {
  const pending = new Map()
  watch(uiDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file)
    if (!path) return
    clearTimeout(pending.get(path))
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiObservers.get(path)
        uiObservers.set(path, observeUiAsset(path))
        previous?.dispose().catch(() => {})
        console.error(`[${WORKER}] reloaded ui asset ${path}`)
      }, 150),
    )
  })
  console.error(`[${WORKER}] serving ui assets from ${uiDir}`)
}

console.log(`${WORKER} worker connected`)

const shutdown = async () => {
  await iii.shutdown()
  process.exit(0)
}
process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
