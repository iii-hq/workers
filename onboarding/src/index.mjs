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

import { existsSync, watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerWorker } from 'iii-sdk'
import { isEmailish, MAX_EMAIL_LENGTH } from './subscribe.mjs'
import { findStep, getTour, listTours } from './tours.mjs'

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
  iii.trigger({
    function_id: 'state::update',
    payload: { scope: STATE_SCOPE, key, ops },
    timeoutMs: STATE_TIMEOUT_MS,
  })

/** The shape the page reads, whether or not anything is stored yet. */
const emptyProgress = () => ({ tours: {}, updated_at: null })

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

/**
 * The topic every completed step is announced on, so anything that wants to
 * follow a tour (the engine's telemetry among them) subscribes instead of
 * reaching into this worker's state.
 */
const STEP_TOPIC = 'onboarding:steps:complete'

/**
 * Published through the `queue` worker, not fire-and-forget pub/sub: a step
 * closes once, and the message waits in the queue and is retried until a
 * subscriber takes it, rather than being dropped when nothing is listening at
 * that instant.
 *
 * The publish itself is still best effort — a missing `queue` worker is
 * logged and never fails the step.
 */
const publishStep = (data) =>
  iii
    .trigger({
      function_id: 'iii::durable::publish',
      payload: { topic: STEP_TOPIC, data },
      timeoutMs: STATE_TIMEOUT_MS,
    })
    .catch((cause) => {
      console.error(`[${WORKER}] could not publish ${STEP_TOPIC}: ${cause?.message ?? cause}`)
    })

iii.registerFunction(
  'onboarding::steps::complete',
  async (input) => {
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
      // Present only when a trigger closed the step, so the page can tell a
      // condition apart from a step the operator closed by hand.
      fired: input.fired
        ? {
            trigger_type: input.fired.trigger_type ?? step.condition?.type ?? null,
            function_id: input.fired.function_id ?? null,
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
// The build half of the tour — a chat of its own, under its own profile
// ---------------------------------------------------------------------------

/** A harness turn is started, not waited out, but the start still crosses the
    directory and the model router. */
const CHAT_TIMEOUT_MS = 30_000

/** Always present (`builtin: true`), so the fallback cannot itself go missing. */
const FALLBACK_AGENT = 'iii-minimal'

/** One session per subject and tour: every step naming an agent joins the
    chat the first one opened. */
const chatKey = (subject, tourId) => `chat:${subject}:${tourId}`

/**
 * The profile to send under, resolved at the moment of the send.
 *
 * Profiles are files in a folder: one can be renamed, edited into a broken
 * `extends` chain, or deleted between one step and the next. So the id is
 * tried first, then the display name, then the bundled profile — the tour
 * asks its question either way, and never stops to tell the operator which
 * identity answered it.
 */
const resolveAgent = async (agent) => {
  const listed = await iii
    .trigger({
      function_id: 'directory::agents::list',
      payload: {},
      timeoutMs: STATE_TIMEOUT_MS,
    })
    .catch(() => null)
  const usable = (listed?.agents ?? []).filter((row) => row && !row.inheritance_error)
  const match = usable.find((row) => row.id === agent.id) ?? usable.find((row) => row.name === agent.name)
  return match?.id ?? FALLBACK_AGENT
}

const harnessSend = (payload) =>
  iii.trigger({ function_id: 'harness::send', payload, timeoutMs: CHAT_TIMEOUT_MS })

iii.registerFunction(
  'onboarding::steps::ask',
  async (input) => {
    const found = findStep(input.tour_id, input.step_id)
    if (!found) throw new Error(`unknown step: ${input.tour_id}/${input.step_id}`)
    const { step } = found
    if (!step.ask || !step.agent) {
      throw new Error(`step sends its own prompt from the console: ${input.tour_id}/${input.step_id}`)
    }
    const subject = input.subject ?? 'local'
    const key = chatKey(subject, input.tour_id)
    const stored = await stateGet(key)
    const existing = typeof stored?.session_id === 'string' ? stored.session_id : null
    if (existing) {
      // A stored session the harness no longer has is not an error the
      // operator can do anything with: the step wants a chat under this
      // profile, so a refused steer opens a new one below.
      const steered = await harnessSend({ session_id: existing, message: step.ask.text }).catch(() => null)
      if (steered) return { session_id: existing, agent: stored.agent ?? null, created: false }
    }
    const agent = await resolveAgent(step.agent)
    // `options.agent` is honoured only on the send that CREATES the session —
    // an identity cannot be retrofitted onto the tour's original chat, which
    // is why this half of the tour is a session of its own. The profile's own
    // model wins when it names one; `model` is the fallback for one that
    // does not, and the page sends the model the operator is already on.
    const started = await harnessSend({
      message: step.ask.text,
      ...(input.model ? { model: input.model } : {}),
      options: { agent },
    })
    await stateUpdate(key, [{ type: 'merge', value: { session_id: started.session_id, agent, at: Date.now() } }])
    return { session_id: started.session_id, agent, created: true }
  },
  {
    description:
      "Send one tour step's prompt to the agent profile that step names, in a chat of its own \u2014 opening that chat on the first such step and steering it on every later one.",
    request_format: object({ subject: string, tour_id: string, step_id: string, model: string }, [
      'tour_id',
      'step_id',
    ]),
    response_format: object(
      { session_id: string, agent: string, created: { type: 'boolean' } },
      ['session_id', 'created'],
    ),
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
const SIGNUP_TIMEOUT_MS = 10_000

const SIGNUP_URL =
  process.env.III_ONBOARDING_SIGNUP_URL ??
  'https://api.mailmodo.com/api/v1/at/f/y0trGR0lfL/c6aefeeb-e66a-5c8a-9c71-4733d9ea1836'

// The operator's address goes over this URL, so an override that downgrades it
// to plain HTTP is refused rather than used.
if (URL.parse(SIGNUP_URL)?.protocol !== 'https:') {
  throw new Error(`III_ONBOARDING_SIGNUP_URL must be an https URL: ${SIGNUP_URL}`)
}

// Mailmodo matches the Origin against an allowlist and this one literal is on
// it. `localhost` and `[::1]` are refused even though they name the same host,
// so the value is fixed rather than derived from wherever the engine happens to
// be listening.
const SIGNUP_ORIGIN = process.env.III_ONBOARDING_SIGNUP_ORIGIN ?? 'http://127.0.0.1'

iii.registerFunction(
  'onboarding::subscribe',
  async (input) => {
    const email = String(input.email ?? '').trim()
    if (!isEmailish(email)) {
      throw new Error(
        `that does not look like an email address (one address, up to ${MAX_EMAIL_LENGTH} characters)`,
      )
    }
    // A signup host that accepts the connection and never answers would hold
    // this invocation open until the caller gives up, with the socket still
    // in hand. The deadline is ours, not the caller's.
    const response = await fetch(SIGNUP_URL, {
      method: 'POST',
      // Mailmodo checks an Origin allowlist and answers 400 "Unauthorized
      // domain" without one. A worker is not a browser, so nothing sets the
      // header for us and we must send it ourselves.
      headers: { 'content-type': 'application/json', origin: SIGNUP_ORIGIN },
      // `data` carries the form fields and must be present: without it the
      // address is refused as "'email' key must be present!", which names the
      // wrong key. Each key here has to match a contact property in Mailmodo
      // or the submission lands with its mapping unresolved, so `email` is the
      // only one we send.
      body: JSON.stringify({ email, data: { email } }),
      // A redirect would carry the address to a host we never checked, so a
      // moved endpoint is an error here rather than a silent second request.
      redirect: 'error',
      signal: AbortSignal.timeout(SIGNUP_TIMEOUT_MS),
    }).catch((cause) => {
      if (cause?.name === 'TimeoutError') throw new Error('the signup service did not answer in time')
      throw cause
    })
    if (!response.ok) {
      throw new Error(`the signup service answered ${response.status}`)
    }
    // Mailmodo answers 200 "added/updated" for an address already on the list,
    // so there is nothing to tell the caller apart from success.
    return { subscribed: true }
  },
  {
    description: 'Add an email address to the iii product-update list.',
    request_format: object({ email: string, source: string }, ['email']),
    response_format: object({ subscribed: { type: 'boolean' } }, ['subscribed']),
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
 * Assets are read from disk at call time rather than inlined at build time, so
 * the watcher below is the same code path as production.
 *
 * Two layouts hold them: beside the published `dist/bundle/index.mjs`, and in
 * `ui/dist` in a source checkout. The first one that has the page wins.
 */
const here = dirname(fileURLToPath(import.meta.url))
const uiDir =
  process.env.III_ONBOARDING_UI_DIR ??
  [here, join(here, '..', 'ui', 'dist')].find((candidate) => existsSync(join(candidate, 'page.js'))) ??
  join(here, '..', 'ui', 'dist')

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
