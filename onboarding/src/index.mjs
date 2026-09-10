/**
 * onboarding — the guided console tour.
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

import { watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerWorker } from 'iii-sdk'
import { getStep, getTour, listTours } from './tours.mjs'

const WORKER = 'onboarding'
const STATE_SCOPE = 'onboarding'
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

iii.registerFunction('onboarding::tours::list', async () => ({ tours: listTours() }), {
  description:
    'List every available tour in curriculum order with its id, title, description, and step count. Call this to find out what onboarding content exists.',
  request_format: object(),
  response_format: object({ tours: { type: 'array' } }, ['tours']),
})

iii.registerFunction(
  'onboarding::tours::get',
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
  iii.trigger({
    function_id: 'state::get',
    payload: { scope: STATE_SCOPE, key },
    timeoutMs: STATE_TIMEOUT_MS,
  })

const stateSet = (key, value) =>
  iii.trigger({
    function_id: 'state::set',
    payload: { scope: STATE_SCOPE, key, value },
    timeoutMs: STATE_TIMEOUT_MS,
  })

/** The shape the page reads, whether or not anything is stored yet. */
const emptyProgress = () => ({ tours: {}, updated_at: null })

const readProgress = async (subject) => {
  const stored = await stateGet(progressKey(subject))
  const progress = stored && typeof stored === 'object' ? stored : emptyProgress()
  return { ...emptyProgress(), ...progress, tours: progress.tours ?? {} }
}

const tourIsComplete = (tour, record) =>
  tour.steps.every((step) => record?.steps?.[step.id]?.status === 'complete')

/**
 * A fired trigger is evidence the page shows back to the operator, so it is
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
  'onboarding::progress::get',
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
      'Read an operator\u2019s tour progress: per tour, the status of every step and the trigger evidence that closed it, plus which tour to offer next.',
    request_format: object({ subject: string }),
    response_format: object(
      { tours: object(), next_tour_id: { type: ['string', 'null'] } },
      ['tours', 'next_tour_id'],
    ),
  },
)

iii.registerFunction(
  'onboarding::steps::complete',
  async (input) => {
    const step = getStep(input.tour_id, input.step_id)
    if (!step) throw new Error(`unknown step: ${input.tour_id}/${input.step_id}`)
    const subject = input.subject ?? 'local'
    const key = progressKey(subject)
    const progress = await readProgress(subject)
    const record = progress.tours[input.tour_id] ?? { steps: {} }
    const steps = { ...(record.steps ?? {}) }
    steps[input.step_id] = {
      status: 'complete',
      at: Date.now(),
      // Present only when a trigger closed the step, so the page can tell a
      // condition apart from a step the operator closed by hand.
      fired: input.fired
        ? {
            trigger_type: input.fired.trigger_type ?? step.condition?.type ?? null,
            function_id: input.fired.function_id ?? null,
            payload: cap(input.fired.payload),
            at: Date.now(),
          }
        : null,
    }
    const tours = {
      ...progress.tours,
      [input.tour_id]: { steps, updated_at: Date.now() },
    }
    await stateSet(key, { tours, updated_at: Date.now() })
    return { step: steps[input.step_id] }
  },
  {
    description:
      'Mark one tour step complete. Pass `fired` when a trigger closed it \u2014 its type and payload are kept as the evidence the tour shows back.',
    request_format: object(
      {
        subject: string,
        tour_id: string,
        step_id: string,
        fired: object({
          trigger_type: string,
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
  'onboarding::steps::reset',
  async (input) => {
    const subject = input.subject ?? 'local'
    const progress = await readProgress(subject)
    const tours = { ...progress.tours }
    delete tours[input.tour_id]
    await stateSet(progressKey(subject), { tours, updated_at: Date.now() })
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

/**
 * The same public signup form the iii.dev landing page posts to (its URL is
 * a meta tag in that page's head, so it is not a secret). The POST happens
 * here and not in the browser: the injected page has no network of its own,
 * and one place to change the destination is enough.
 */
const SIGNUP_URL =
  process.env.III_ONBOARDING_SIGNUP_URL ??
  'https://api.mailmodo.com/api/v1/at/f/b7XMGvRS9B/cdb51f52-a91e-520c-888d-03470a9c8faa'

iii.registerFunction(
  'onboarding::subscribe',
  async (input) => {
    const email = String(input.email ?? '').trim()
    // One address, an @, and a dot after it. The list itself does the real
    // validation; this only stops an obvious typo becoming a POST.
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) throw new Error('that does not look like an email address')
    const response = await fetch(SIGNUP_URL, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ email, source: input.source ?? 'onboarding_flow' }),
    })
    // 409 is "already on the list", which is a success for the operator.
    if (!response.ok && response.status !== 409) {
      throw new Error(`the signup service answered ${response.status}`)
    }
    return { subscribed: true, already: response.status === 409 }
  },
  {
    description: 'Add an email address to the iii product-update list.',
    request_format: object({ email: string, source: string }, ['email']),
    response_format: object({ subscribed: { type: 'boolean' }, already: { type: 'boolean' } }, ['subscribed']),
  },
)

// ---------------------------------------------------------------------------
// Injected console UI
// ---------------------------------------------------------------------------

const uiAssets = {
  'onboarding/page.js': {
    file: 'page.js',
    type: 'console:script',
    content_type: 'text/javascript',
  },
  'onboarding/styles.css': {
    file: 'styles.css',
    type: 'console:style',
    content_type: 'text/css',
  },
}

/**
 * Assets are read from `ui/dist` at call time rather than inlined at build
 * time, so the watcher below is the same code path as production.
 */
const uiDir =
  process.env.III_ONBOARDING_UI_DIR ??
  join(dirname(fileURLToPath(import.meta.url)), '..', 'ui', 'dist')

iii.registerFunction(
  'onboarding::ui-content',
  async (input) => {
    const asset = uiAssets[input.path]
    if (!asset) throw new Error(`unknown ui asset: ${input.path}`)
    const content = await readFile(join(uiDir, asset.file), 'utf8')
    return { content, content_type: asset.content_type }
  },
  {
    description: 'Serve the injectable onboarding console page assets.',
    metadata: { internal: true },
    request_format: object({ path: string }, ['path']),
    response_format: object({ content: string, content_type: string }, [
      'content',
      'content_type',
    ]),
  },
)

const registerUiAsset = (path) =>
  iii.registerTrigger({
    type: uiAssets[path].type,
    function_id: 'onboarding::ui-content',
    config: { path },
  })

const uiTriggers = new Map(
  Object.keys(uiAssets).map((path) => [path, registerUiAsset(path)]),
)

// Re-registering a path with different bytes hot-swaps the asset in every open
// console tab, so editing the page is one rebuild away from being visible.
if (process.env.III_ONBOARDING_UI_WATCH) {
  const pending = new Map()
  watch(uiDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file)
    if (!path) return
    clearTimeout(pending.get(path))
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiTriggers.get(path)
        uiTriggers.set(path, registerUiAsset(path))
        previous?.unregister()
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
