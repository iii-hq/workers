import { describe, expect, it } from 'vitest'
import {
  addToStack,
  defaultDirectory,
  entryFile,
  type ListTemplatesResult,
  MAX_POLLS,
  NEW_WORKER_INITIAL,
  type NewWorkerAction,
  type NewWorkerState,
  newWorkerReducer,
  pickTemplate,
  type ScaffoldResult,
  type StackPhase,
  sourceLabel,
  templatePlaceholder,
  type Trigger,
  validateWorkerName,
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
    // Radix's native select reports '' while its options are not yet mounted.
    expect(pickTemplate('', templates)).toBe('worker-node-ade')
    expect(pickTemplate('worker-python', templates)).toBe('worker-python')
    // A pick that a Refresh dropped from the list.
    expect(pickTemplate('gone', templates)).toBe('worker-node-ade')
    expect(pickTemplate(undefined, templates.slice(0, 1))).toBe('worker-python')
    expect(pickTemplate('worker-python', [])).toBeUndefined()
  })
})

describe('templatePlaceholder', () => {
  it('says Loading while loading, Choose a template once some loaded, No templates when none', () => {
    expect(templatePlaceholder(NEW_WORKER_INITIAL)).toBe('Loading…')
    expect(templatePlaceholder({ ...NEW_WORKER_INITIAL, step: 'form', list: LIST })).toBe('Choose a template')
    expect(templatePlaceholder({ ...NEW_WORKER_INITIAL, step: 'form', list: { ...LIST, templates: [] } })).toBe(
      'No templates',
    )
    expect(templatePlaceholder({ ...NEW_WORKER_INITIAL, step: 'form', error: 'boom' })).toBe('No templates')
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
    expect(run([{ type: 'add' }], failedAdd)).toMatchObject({ step: 'adding', phase: 'installing', error: null, logs: [] })

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
// repeats for every later call.
function bus(replies: Record<string, unknown[]>) {
  const calls: Array<[string, Record<string, unknown>]> = []
  const trigger: Trigger = async <T>(functionId: string, payload: Record<string, unknown>): Promise<T> => {
    calls.push([functionId, payload])
    const queue = replies[functionId]
    if (queue === undefined || queue.length === 0) throw new Error(`unexpected call ${functionId}`)
    const next = queue.length > 1 ? queue.shift() : queue[0]
    if (next instanceof Error) throw next
    return next as T
  }
  const count = (functionId: string) => calls.filter(([fn]) => fn === functionId).length
  const payload = (functionId: string) => calls.find(([fn]) => fn === functionId)?.[1]
  return { trigger, calls, count, payload }
}

const status = (...rows: Array<[string, string, string?]>) => ({
  containers: rows.map(([container, state, lastError]) => ({ container, state, last_error: lastError ?? null })),
})
const accepted = (id: string) => ({ operation_id: id, requested: 1, status: 'accepted' })
const noWait = async () => {}

describe('addToStack', () => {
  it('adds the worker with the missing http, then follows it to ready', async () => {
    const fake = bus({
      'compose::status': [
        status(['ide', 'ready']),
        status(['ide', 'ready']),
        status(['ide', 'ready'], ['my-thing', 'starting']),
        status(['ide', 'ready'], ['my-thing', 'ready']),
      ],
      'compose::add': [accepted('op-1')],
      'compose::operation': [{ status: 'running' }],
    })
    const progress: StackPhase[] = []
    const outcome = await addToStack(fake.trigger, RESULT, (phase) => progress.push(phase), { sleep: noWait })
    expect(outcome).toEqual({ ok: true })
    expect(fake.payload('compose::status')).toEqual({})
    // The same http declaration as the -ade templates' worker-compose.yaml.
    expect(fake.payload('compose::add')).toEqual({
      workers: [RESULT.compose, { worker: 'package://http', version: 'latest', config_name: 'http' }],
    })
    expect(fake.payload('compose::operation')).toEqual({ progress_operation_id: 'op-1' })
    expect(progress).toEqual(['installing', 'starting'])
  })

  it('adds only the worker when the stack already runs http', async () => {
    const fake = bus({
      'compose::status': [status(['http', 'ready']), status(['http', 'ready'], ['my-thing', 'ready'])],
      'compose::add': [accepted('op-2')],
      'compose::operation': [{ status: 'running' }],
    })
    expect(await addToStack(fake.trigger, RESULT, () => {}, { sleep: noWait })).toEqual({ ok: true })
    expect(fake.payload('compose::add')).toEqual({ workers: [RESULT.compose] })
  })

  it('refuses a name the stack already has, before compose::add', async () => {
    // compose::add would repoint the existing my-thing container at the new folder.
    const fake = bus({ 'compose::status': [status(['ide', 'ready'], ['my-thing', 'ready'])] })
    const outcome = await addToStack(fake.trigger, RESULT, () => {}, { sleep: noWait })
    expect(outcome).toEqual({
      ok: false,
      error: 'a container named my-thing already exists in the stack',
      logs: [],
      owned: false,
    })
    expect(fake.calls.map(([fn]) => fn)).toEqual(['compose::status'])
  })

  it('returns the error and the log tail of a container that failed', async () => {
    const fake = bus({
      'compose::status': [status(), status(['my-thing', 'failed', 'pre_run exited with status 1'])],
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
    const outcome = await addToStack(fake.trigger, RESULT, () => {}, { sleep: noWait })
    expect(outcome).toEqual({
      ok: false,
      error: 'pre_run exited with status 1',
      logs: ['npm ERR! missing script: build'],
      owned: true,
    })
    expect(fake.payload('compose::logs')).toEqual({ container: 'my-thing', tail: 40 })
  })

  it('stops waiting when the add operation fails before the container appears', async () => {
    const fake = bus({
      'compose::status': [status()],
      'compose::add': [accepted('op-4')],
      'compose::operation': [{ status: 'failed', last_event: { detail: 'could not resolve http' } }],
      'compose::logs': [new Error('UNKNOWN_CONTAINER')],
    })
    const progress: StackPhase[] = []
    const outcome = await addToStack(fake.trigger, RESULT, (phase) => progress.push(phase), { sleep: noWait })
    expect(outcome).toEqual({ ok: false, error: 'could not resolve http', logs: [], owned: true })
    expect(progress).toEqual([])
  })

  it('restarts the container a failed attempt added, on retry', async () => {
    const fake = bus({
      'compose::status': [status(['my-thing', 'failed']), status(['my-thing', 'ready'])],
      'compose::restart': [{ status: 'ok', changed: true }],
    })
    const outcome = await addToStack(fake.trigger, RESULT, () => {}, { owned: true, sleep: noWait })
    expect(outcome).toEqual({ ok: true })
    expect(fake.payload('compose::restart')).toEqual({ container: 'my-thing' })
    expect(fake.count('compose::add')).toBe(0)
    expect(fake.count('compose::operation')).toBe(0)
  })

  it('keeps owned on a retry whose first status read fails', async () => {
    // The dialog stores this owned and passes it to the next Retry; losing it would
    // stop that Retry at "already exists in the stack" for good.
    const fake = bus({ 'compose::status': [new Error('bus down')], 'compose::logs': [new Error('UNKNOWN_CONTAINER')] })
    const outcome = await addToStack(fake.trigger, RESULT, () => {}, { owned: true, sleep: noWait })
    expect(outcome).toEqual({ ok: false, error: 'bus down', logs: [], owned: true })
    // A first attempt that never reached compose still owns nothing.
    const first = bus({ 'compose::status': [new Error('bus down')], 'compose::logs': [new Error('UNKNOWN_CONTAINER')] })
    expect(await addToStack(first.trigger, RESULT, () => {}, { sleep: noWait })).toMatchObject({ owned: false })
  })

  it('gives up after MAX_POLLS reads', async () => {
    const fake = bus({
      'compose::status': [status(), status(['my-thing', 'starting'])],
      'compose::add': [accepted('op-5')],
      'compose::operation': [{ status: 'running' }],
      'compose::logs': [{ containers: [] }],
    })
    const outcome = await addToStack(fake.trigger, RESULT, () => {}, { sleep: noWait })
    expect(outcome).toEqual({ ok: false, error: 'my-thing did not start within 10 minutes.', logs: [], owned: true })
    expect(fake.count('compose::status')).toBe(MAX_POLLS + 1)
  })
})
