import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  addToStack,
  composeOperations,
  defaultDirectory,
  type EventClient,
  entryFile,
  followStart,
  formatElapsed,
  GIVE_UP_MS,
  type ListTemplatesResult,
  NEW_WORKER_INITIAL,
  type NewWorkerAction,
  type NewWorkerState,
  newWorkerReducer,
  type ProgressEvent,
  pickTemplate,
  type ScaffoldResult,
  type StackPhase,
  type Subscribe,
  sourceLabel,
  stackSteps,
  type Trigger,
  templateChoices,
  validateWorkerName,
  workerFunctions,
} from '../new-worker'

const LIST: ListTemplatesResult = {
  source: {
    kind: 'git',
    location: 'https://github.com/iii-hq/templates.git',
    ref: 'main',
    revision: 'abc1234def567',
    warning: null,
  },
  templates: [
    { id: 'worker-node-ade', name: 'Worker (Node, ADE)', description: 'node', language: 'node', requires: ['http'] },
  ],
}

const RESULT: ScaffoldResult = {
  template: 'worker-node-ade',
  name: 'my-thing',
  directory: '/repo/workers/my-thing',
  files: [
    { path: '/repo/workers/my-thing/package.json', bytes: 10, revision: 'r1' },
    { path: '/repo/workers/my-thing/src/index.ts', bytes: 20, revision: 'r2' },
  ],
  compose: { worker: '/repo/workers/my-thing', scripts: { pre_run: 'pnpm install', run: 'pnpm dev' } },
  requires: ['http'],
  next_steps: [],
}

describe('validateWorkerName', () => {
  it('accepts kebab names of 1 to 63 characters', () => {
    for (const name of ['a', 'my-worker', 'a1-b2', 'a-1', 'a'.repeat(63)]) {
      expect(validateWorkerName(name)).toBeNull()
    }
  })

  it('rejects everything else with a reason', () => {
    expect(validateWorkerName('')).toBe('Enter a name.')
    expect(validateWorkerName('a'.repeat(64))).toBe('At most 63 characters.')
    expect(validateWorkerName('a--b')).toBe('Lowercase letters, digits and single hyphens, starting with a letter.')
    for (const name of ['-a', 'a-', 'My-worker', '1a', 'my_worker', 'my worker', 'é']) {
      expect(validateWorkerName(name)).toBe('Lowercase letters, digits and single hyphens, starting with a letter.')
    }
  })
})

describe('defaultDirectory', () => {
  it('puts the worker folder, named after the worker, under the parent folder', () => {
    expect(defaultDirectory('', 'svc')).toBe('svc')
    expect(defaultDirectory('workers', 'svc')).toBe('workers/svc')
    expect(defaultDirectory('apps/backend/', 'svc')).toBe('apps/backend/svc')
    expect(defaultDirectory('/tmp/scratch', 'svc')).toBe('/tmp/scratch/svc')
    // The absolute root stays absolute: stripDirSlash('/') is '' and would turn it root-relative.
    expect(defaultDirectory('/', 'svc')).toBe('/svc')
  })
})

describe('entryFile', () => {
  it('picks the shallowest src/index.ts or src/main.py', () => {
    expect(entryFile(RESULT.files.map((file) => file.path))).toBe('/repo/workers/my-thing/src/index.ts')
    expect(entryFile(['/r/w/py/pyproject.toml', '/r/w/py/ui/src/main.py', '/r/w/py/src/main.py'])).toBe(
      '/r/w/py/src/main.py',
    )
    expect(entryFile(['/r/w/x/README.md'])).toBeNull()
  })
})

describe('sourceLabel', () => {
  it('names the repo, ref and short revision, or the local folder', () => {
    expect(sourceLabel(LIST.source)).toBe('iii-hq/templates@main · abc1234')
    expect(sourceLabel({ kind: 'dir', location: '/home/me/templates/iii', revision: null, warning: null })).toBe(
      'local: /home/me/templates/iii',
    )
  })
})

describe('pickTemplate', () => {
  const info = (id: string, language: 'node' | 'python') => ({ id, name: id, description: '', language, requires: [] })
  const templates = [info('worker-python', 'python'), info('worker-node-ade', 'node')]

  it('keeps a pick that is listed, else falls back to the first -ade template, else the first', () => {
    expect(pickTemplate(undefined, templates)).toBe('worker-node-ade')
    expect(pickTemplate('', templates)).toBe('worker-node-ade')
    expect(pickTemplate('worker-python', templates)).toBe('worker-python')
    // A pick that a Refresh dropped from the list.
    expect(pickTemplate('gone', templates)).toBe('worker-node-ade')
    expect(pickTemplate(undefined, templates.slice(0, 1))).toBe('worker-python')
    expect(pickTemplate('worker-python', [])).toBeUndefined()
  })
})

describe('templateChoices', () => {
  const info = (id: string, name: string, language: 'node' | 'python') => ({
    id,
    name,
    description: '',
    language,
    requires: [],
  })

  it('lists one language, -ade first, titled without the language', () => {
    const templates = [
      info('worker-python', 'Worker (Python)', 'python'),
      info('worker-python-ade', 'Worker with ADE page (Python)', 'python'),
      info('worker-node', 'Worker (Node)', 'node'),
      info('custom', 'Custom', 'python'),
    ]
    expect(templateChoices(templates, 'python').map((t) => [t.id, t.title])).toEqual([
      ['worker-python-ade', 'Worker with ADE page'],
      ['worker-python', 'Worker'],
      ['custom', 'Custom'],
    ])
    expect(templateChoices(templates, 'node').map((t) => t.title)).toEqual(['Worker'])
  })
})

describe('newWorkerReducer', () => {
  const run = (actions: NewWorkerAction[], from: NewWorkerState = NEW_WORKER_INITIAL) =>
    actions.reduce(newWorkerReducer, from)

  it('walks loading → form → creating → result → adding → running', () => {
    const form = run([{ type: 'listed', list: LIST }])
    expect(form).toMatchObject({ step: 'form', list: LIST, error: null })
    const creating = run([{ type: 'create' }], form)
    expect(creating.step).toBe('creating')
    const result = run([{ type: 'created', result: RESULT }], creating)
    expect(result).toMatchObject({ step: 'result', result: RESULT })
    const adding = run([{ type: 'add' }, { type: 'progress', phase: 'starting' }], result)
    expect(adding).toMatchObject({ step: 'adding', phase: 'starting' })
    expect(run([{ type: 'added' }], adding).step).toBe('running')
  })

  it('keeps errors in place and lets a failed step run again', () => {
    const failedCreate = run([
      { type: 'listed', list: LIST },
      { type: 'create' },
      { type: 'create-failed', error: 'C233 target not empty' },
    ])
    expect(failedCreate).toMatchObject({ step: 'form', error: 'C233 target not empty' })
    expect(run([{ type: 'create' }], failedCreate)).toMatchObject({ step: 'creating', error: null })

    const failedAdd = run(
      [
        { type: 'create' },
        { type: 'created', result: RESULT },
        { type: 'add' },
        { type: 'add-failed', error: 'boom', logs: ['npm ERR!'], owned: true },
      ],
      failedCreate,
    )
    expect(failedAdd).toMatchObject({ step: 'failed', error: 'boom', logs: ['npm ERR!'], owned: true })
    expect(run([{ type: 'add' }], failedAdd)).toMatchObject({
      step: 'adding',
      phase: 'installing',
      error: null,
      logs: [],
    })

    const listFailed = run([{ type: 'list-failed', error: 'C230 templates unavailable' }])
    expect(listFailed).toMatchObject({ step: 'form', list: null, error: 'C230 templates unavailable' })
    expect(run([{ type: 'load' }], listFailed)).toMatchObject({ step: 'loading', error: null })
  })

  it('ignores what does not fit the step', () => {
    expect(run([{ type: 'create' }])).toBe(NEW_WORKER_INITIAL)
    const result = run([{ type: 'listed', list: LIST }, { type: 'create' }, { type: 'created', result: RESULT }])
    const stray: NewWorkerAction[] = [
      { type: 'listed', list: LIST },
      { type: 'load' },
      { type: 'create' },
      { type: 'progress', phase: 'starting' },
      { type: 'added' },
    ]
    for (const action of stray) expect(newWorkerReducer(result, action)).toBe(result)
  })
})

// A fake bus: each function answers from its own queue, and the last answer
// repeats for every later call. An answer that is a function is called with
// the payload (to look at what is bound at that moment).
function bus(replies: Record<string, unknown[]>) {
  const calls: Array<[string, Record<string, unknown>]> = []
  const trigger: Trigger = async <T>(functionId: string, payload: Record<string, unknown>): Promise<T> => {
    calls.push([functionId, payload])
    const queue = replies[functionId]
    if (queue === undefined || queue.length === 0) throw new Error(`unexpected call ${functionId}`)
    let next = queue.length > 1 ? queue.shift() : queue[0]
    if (typeof next === 'function') next = next(payload)
    if (next instanceof Error) throw next
    return next as T
  }
  const count = (functionId: string) => calls.filter(([fn]) => fn === functionId).length
  const payload = (functionId: string) => calls.find(([fn]) => fn === functionId)?.[1]
  return { trigger, calls, count, payload }
}

// A fake compose-operation feed: what is bound, and events sent to it.
function feed() {
  const bound = new Map<string, (event: ProgressEvent) => void>()
  const subscribe: Subscribe = (operationId, onEvent) => {
    bound.set(operationId, onEvent)
    return () => bound.delete(operationId)
  }
  const emit = (operationId: string, event: Partial<ProgressEvent>) =>
    bound.get(operationId)?.({ operation_id: operationId, phase: 'complete', detail: '', terminal: false, ...event })
  return { bound, subscribe, emit }
}

const status = (...rows: Array<[string, string, string?]>) => ({
  containers: rows.map(([container, state, lastError]) => ({ container, state, last_error: lastError ?? null })),
})
const accepted = (id: string) => ({ operation_id: id, requested: 1, status: 'accepted' })
const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('stackSteps', () => {
  const at = (step: NewWorkerState['step'], phase: StackPhase = 'installing') =>
    stackSteps({ ...NEW_WORKER_INITIAL, step, phase }).map((s) => `${s.label}:${s.state}`)

  it('follows Add to stack and names the step a failure stopped at', () => {
    expect(at('result')).toEqual(['Install:pending', 'Start:pending'])
    expect(at('adding')).toEqual(['Installing…:active', 'Start:pending'])
    expect(at('adding', 'starting')).toEqual(['Installed:done', 'Starting…:active'])
    expect(at('running', 'starting')).toEqual(['Installed:done', 'Running:live'])
    expect(at('failed')).toEqual(['Install failed:failed', 'Not started:skipped'])
    expect(at('failed', 'starting')).toEqual(['Installed:done', 'Did not start:failed'])
  })
})

describe('formatElapsed', () => {
  it('counts seconds, then minutes and padded seconds', () => {
    expect([0, 8_400, 59_999, 65_000, 600_000].map(formatElapsed)).toEqual(['0s', '8s', '59s', '1m 05s', '10m 00s'])
  })
})

describe('workerFunctions', () => {
  it('keeps the worker’s own ids, sorted, and not a prefix-sharing worker’s', () => {
    const entries = [
      { function_id: 'orders::hello' },
      { function_id: 'orders-v2::hello' },
      { function_id: 'state::get' },
      { function_id: 'orders::archive', description: 'Archive one' },
    ]
    expect(workerFunctions(entries, 'orders').map((f) => f.function_id)).toEqual(['orders::archive', 'orders::hello'])
  })
})

describe('addToStack', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('binds the operation before compose::add, then follows its events to ready', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status(['ide', 'ready'])],
      'compose::add': [
        () => {
          // Bound first: no event of the operation falls before the binding.
          expect([...events.bound.keys()]).toEqual(['op-1'])
          return accepted('op-1')
        },
      ],
      'compose::operation': [{ status: 'running' }],
    })
    const progress: StackPhase[] = []
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, (phase) => progress.push(phase), {
      operationId: 'op-1',
    })
    await flush()
    // The same http declaration as the -ade templates' worker-compose.yaml.
    expect(fake.payload('compose::add')).toEqual({
      workers: [RESULT.compose, { worker: 'package://http', version: 'latest', config_name: 'http' }],
      operation_id: 'op-1',
    })
    // One catch-up read of the snapshot, and nothing read again after it.
    expect(fake.payload('compose::operation')).toEqual({ progress_operation_id: 'op-1' })
    events.emit('op-1', { container: 'http', phase: 'ready' })
    events.emit('op-1', { container: 'my-thing', phase: 'starting' })
    events.emit('op-1', { container: 'my-thing', phase: 'ready' })
    expect(await pending).toEqual({ ok: true })
    expect(progress).toEqual(['installing', 'starting'])
    expect(fake.calls.map(([fn]) => fn)).toEqual(['compose::status', 'compose::add', 'compose::operation'])
    expect(events.bound.size).toBe(0)
  })

  it('adds only the worker when the stack already runs http', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status(['http', 'ready'])],
      'compose::add': [accepted('op-2')],
      'compose::operation': [{ status: 'running' }],
    })
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, () => {}, { operationId: 'op-2' })
    await flush()
    events.emit('op-2', { container: 'my-thing', phase: 'ready' })
    expect(await pending).toEqual({ ok: true })
    expect(fake.payload('compose::add')).toEqual({ workers: [RESULT.compose], operation_id: 'op-2' })
  })

  it('picks its own operation id when none is given', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status(['http', 'ready'])],
      'compose::add': [accepted('whatever')],
      'compose::operation': [{ status: 'running' }],
    })
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, () => {})
    await flush()
    const [id] = [...events.bound.keys()]
    expect(id).toMatch(/^compose:[0-9a-f-]{36}$/)
    expect(fake.payload('compose::add')?.operation_id).toBe(id)
    events.emit(id, { container: 'my-thing', phase: 'ready' })
    expect(await pending).toEqual({ ok: true })
  })

  it('refuses a name the stack already has, before compose::add', async () => {
    // compose::add would repoint the existing my-thing container at the new folder.
    const events = feed()
    const fake = bus({ 'compose::status': [status(['ide', 'ready'], ['my-thing', 'ready'])] })
    const outcome = await addToStack(fake.trigger, events.subscribe, RESULT, () => {})
    expect(outcome).toEqual({
      ok: false,
      error: 'a container named my-thing already exists in the stack',
      logs: [],
      owned: false,
    })
    expect(fake.calls.map(([fn]) => fn)).toEqual(['compose::status'])
    expect(events.bound.size).toBe(0)
  })

  it('returns the error and the log tail of a container that failed', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status()],
      'compose::add': [accepted('op-3')],
      'compose::operation': [{ status: 'running' }],
      'compose::logs': [
        {
          containers: [
            {
              container: 'my-thing',
              entries: [{ message: 'npm ERR! missing script: build\n', stream: 'stderr' }],
              truncated: false,
            },
          ],
        },
      ],
    })
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, () => {}, { operationId: 'op-3' })
    await flush()
    events.emit('op-3', { container: 'my-thing', phase: 'failed', detail: 'pre_run exited with status 1' })
    expect(await pending).toEqual({
      ok: false,
      error: 'pre_run exited with status 1',
      logs: ['npm ERR! missing script: build'],
      owned: true,
    })
    expect(fake.payload('compose::logs')).toEqual({ container: 'my-thing', tail: 40 })
    expect(events.bound.size).toBe(0)
  })

  it('stops at an add operation that failed before the binding heard it', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status()],
      'compose::add': [accepted('op-4')],
      'compose::operation': [{ status: 'failed', last_event: { detail: 'could not resolve http' } }],
      'compose::logs': [new Error('UNKNOWN_CONTAINER')],
    })
    const progress: StackPhase[] = []
    const outcome = await addToStack(fake.trigger, events.subscribe, RESULT, (phase) => progress.push(phase), {
      operationId: 'op-4',
    })
    expect(outcome).toEqual({ ok: false, error: 'could not resolve http', logs: [], owned: true })
    expect(progress).toEqual([])
    expect(events.bound.size).toBe(0)
  })

  it('reads the verdict once when the operation’s terminal event arrives', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status(), status(['my-thing', 'starting'])],
      'compose::add': [accepted('op-5')],
      'compose::operation': [
        { status: 'running' },
        { status: 'succeeded', last_event: { detail: 'my-thing is not required and did not start' } },
      ],
      'compose::logs': [{ containers: [] }],
    })
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, () => {}, { operationId: 'op-5' })
    await flush()
    events.emit('op-5', { phase: 'complete', terminal: true })
    expect(await pending).toEqual({
      ok: false,
      error: 'my-thing is not required and did not start',
      logs: [],
      owned: true,
    })
    expect(fake.count('compose::operation')).toBe(2)
    expect(fake.count('compose::status')).toBe(2)
  })

  it('restarts the container a failed attempt added, on retry', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status(['my-thing', 'failed']), status(['my-thing', 'ready'])],
      'compose::restart': [{ status: 'ok', changed: true }],
    })
    const progress: StackPhase[] = []
    const outcome = await addToStack(fake.trigger, events.subscribe, RESULT, (phase) => progress.push(phase), {
      owned: true,
    })
    expect(outcome).toEqual({ ok: true })
    expect(progress).toEqual(['starting'])
    expect(fake.payload('compose::restart')).toEqual({ container: 'my-thing' })
    expect(fake.count('compose::add')).toBe(0)
    expect(fake.count('compose::operation')).toBe(0)
  })

  it('keeps owned on a retry whose first status read fails', async () => {
    // The dialog stores this owned and passes it to the next Retry; losing it would
    // stop that Retry at "already exists in the stack" for good.
    const events = feed()
    const fake = bus({ 'compose::status': [new Error('bus down')], 'compose::logs': [new Error('UNKNOWN_CONTAINER')] })
    const outcome = await addToStack(fake.trigger, events.subscribe, RESULT, () => {}, { owned: true })
    expect(outcome).toEqual({ ok: false, error: 'bus down', logs: [], owned: true })
    // A first attempt that never reached compose still owns nothing.
    const first = bus({ 'compose::status': [new Error('bus down')], 'compose::logs': [new Error('UNKNOWN_CONTAINER')] })
    expect(await addToStack(first.trigger, events.subscribe, RESULT, () => {})).toMatchObject({ owned: false })
  })

  it('gives up once, after GIVE_UP_MS without a verdict, reading nothing meanwhile', async () => {
    vi.useFakeTimers()
    const events = feed()
    const fake = bus({
      'compose::status': [status()],
      'compose::add': [accepted('op-6')],
      'compose::operation': [{ status: 'running' }],
      'compose::logs': [{ containers: [] }],
    })
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, () => {}, { operationId: 'op-6' })
    await vi.advanceTimersByTimeAsync(GIVE_UP_MS - 1)
    expect(fake.calls.map(([fn]) => fn)).toEqual(['compose::status', 'compose::add', 'compose::operation'])
    await vi.advanceTimersByTimeAsync(1)
    expect(await pending).toEqual({
      ok: false,
      error: 'my-thing did not start within 10 minutes.',
      logs: [],
      owned: true,
    })
    expect(events.bound.size).toBe(0)
  })

  it('stops following, and unbinds, when the dialog closes', async () => {
    const events = feed()
    const fake = bus({
      'compose::status': [status()],
      'compose::add': [accepted('op-7')],
      'compose::operation': [{ status: 'running' }],
    })
    const controller = new AbortController()
    const pending = addToStack(fake.trigger, events.subscribe, RESULT, () => {}, {
      operationId: 'op-7',
      signal: controller.signal,
    })
    await flush()
    expect(events.bound.size).toBe(1)
    controller.abort()
    expect(await pending).toMatchObject({ ok: false, owned: true })
    expect(events.bound.size).toBe(0)
  })
})

describe('followStart', () => {
  it('follows a scaffold-started worker through its operation to ready', async () => {
    const events = feed()
    const fake = bus({ 'compose::operation': [{ status: 'running' }] })
    const progress: StackPhase[] = []
    const pending = followStart(fake.trigger, events.subscribe, 'my-thing', 'op-1', (phase) => progress.push(phase))
    await flush()
    expect(fake.payload('compose::operation')).toEqual({ progress_operation_id: 'op-1' })
    events.emit('op-1', { container: 'my-thing', phase: 'registering' })
    events.emit('op-1', { container: 'my-thing', phase: 'ready' })
    expect(await pending).toEqual({ ok: true })
    expect(progress).toEqual(['installing', 'starting'])
    expect(fake.count('compose::status')).toBe(0)
  })

  it('answers from the snapshot when the operation ended before the binding', async () => {
    const events = feed()
    const fake = bus({
      'compose::operation': [{ status: 'succeeded' }],
      'compose::status': [status(['my-thing', 'ready'])],
    })
    expect(await followStart(fake.trigger, events.subscribe, 'my-thing', 'op-1', () => {})).toEqual({ ok: true })
  })

  it('answers from the container once compose no longer knows the operation', async () => {
    // An old chat card: the operation is gone, the worker still runs.
    const events = feed()
    const fake = bus({
      'compose::operation': [new Error('operation not found')],
      'compose::status': [status(['my-thing', 'ready'])],
    })
    expect(await followStart(fake.trigger, events.subscribe, 'my-thing', 'op-old', () => {})).toEqual({ ok: true })
    expect(fake.count('compose::operation')).toBe(1)
  })

  it('reports a worker that is in no operation and not in the stack', async () => {
    const events = feed()
    const fake = bus({
      'compose::operation': [new Error('operation not found')],
      'compose::status': [status(['ide', 'ready'])],
      'compose::logs': [new Error('no container')],
    })
    expect(await followStart(fake.trigger, events.subscribe, 'my-thing', 'op-old', () => {})).toEqual({
      ok: false,
      error: 'my-thing is not in the stack.',
      logs: [],
      owned: true,
    })
  })
})

describe('composeOperations', () => {
  it('binds compose-operation on the operation, hears only it, and unbinds', () => {
    const handlers = new Map<string, (payload: unknown) => void>()
    const triggers: Array<{ type: string; function_id: string; config: Record<string, unknown> }> = []
    let unbound = 0
    const client: EventClient = {
      browserId: 'console-1',
      on: (functionId, handler) => {
        handlers.set(functionId, handler as (payload: unknown) => void)
        return () => handlers.delete(functionId)
      },
      registerTrigger: (input) => {
        triggers.push(input)
        return () => {
          unbound += 1
        }
      },
    }
    const heard: string[] = []
    const off = composeOperations(client)('op-9', (event) => heard.push(event.phase))
    expect(triggers).toHaveLength(1)
    const [binding] = triggers
    expect(binding.type).toBe('compose-operation')
    expect(binding.config).toEqual({ operation_id: 'op-9' })
    const [functionId] = [...handlers.keys()]
    expect(functionId.startsWith('iii::shell-ui::compose-operation::')).toBe(true)
    expect(binding.function_id).toBe(`${functionId}::console-1`)
    handlers.get(functionId)?.({ operation_id: 'op-9', phase: 'starting', detail: '', terminal: false })
    handlers.get(functionId)?.({ operation_id: 'other', phase: 'ready', detail: '', terminal: false })
    expect(heard).toEqual(['starting'])
    off()
    expect(unbound).toBe(1)
    expect(handlers.size).toBe(0)
  })
})
