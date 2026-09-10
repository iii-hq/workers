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
import { getTour, listTours } from './tours.mjs'

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

iii.registerFunction(
  'onboarding::progress::get',
  async (input) => {
    const stored = await stateGet(progressKey(input?.subject ?? 'local'))
    const progress = stored && typeof stored === 'object' ? stored : emptyProgress()
    const tours = progress.tours ?? {}
    const next = listTours().find((tour) => tours[tour.id]?.status !== 'completed')
    return { ...emptyProgress(), ...progress, tours, next_tour_id: next?.id ?? null }
  },
  {
    description:
      'Read how far an operator got: per tour, the furthest step reached and whether it is completed, plus which tour to offer next.',
    request_format: object({ subject: string }),
    response_format: object(
      { tours: object(), next_tour_id: { type: ['string', 'null'] } },
      ['tours', 'next_tour_id'],
    ),
  },
)

iii.registerFunction(
  'onboarding::progress::set',
  async (input) => {
    const tour = getTour(input.tour_id)
    if (!tour) throw new Error(`unknown tour: ${input.tour_id}`)
    const key = progressKey(input.subject ?? 'local')
    const stored = await stateGet(key)
    const progress = stored && typeof stored === 'object' ? stored : emptyProgress()
    const tours = { ...(progress.tours ?? {}) }
    const previous = tours[input.tour_id]
    const step_index = Math.max(0, Math.min(input.step_index, tour.steps.length - 1))
    tours[input.tour_id] = {
      step_index,
      // Furthest step ever reached, so re-reading an earlier step does not
      // re-lock the ones after it.
      reached_index: Math.max(step_index, previous?.reached_index ?? 0),
      status: input.status ?? 'started',
      updated_at: Date.now(),
    }
    const next = { tours, updated_at: Date.now() }
    await stateSet(key, next)
    return { progress: next.tours[input.tour_id] }
  },
  {
    description:
      'Record which step of a tour an operator is on. The console page sends one of these per step change, completion, and dismissal.',
    request_format: object(
      {
        subject: string,
        tour_id: string,
        step_index: integer,
        status: { type: 'string', enum: ['started', 'completed', 'dismissed'] },
      },
      ['tour_id', 'step_index'],
    ),
    response_format: object({ progress: object() }, ['progress']),
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
