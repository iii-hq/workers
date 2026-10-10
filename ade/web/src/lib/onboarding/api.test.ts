import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  checkProviderKey,
  composeEventProgress,
  judgeFunctionsSignature,
  runStep,
  type StepProgress,
} from './api'
import type { PlanStep } from './plan'

const trigger = vi.fn()

/** A fake engine: browser-local handlers plus the trigger bindings on them. */
interface Binding {
  type: string
  function_id: string
  config: Record<string, unknown>
  active: boolean
}
const handlers = new Map<string, (payload: unknown) => void>()
const bindings: Binding[] = []
/** Subscriptions and calls in the order they happened. */
const order: string[] = []
const connected = new Set<string>()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    trigger,
    browserId: 'tab',
    on: (id: string, handler: (payload: unknown) => void) => {
      handlers.set(`${id}::tab`, handler)
      return () => handlers.delete(`${id}::tab`)
    },
    registerTrigger: (input: Omit<Binding, 'active'>) => {
      const binding = { ...input, active: true }
      bindings.push(binding)
      order.push(`subscribe ${input.type}`)
      return () => {
        binding.active = false
      }
    },
  }),
}))

vi.mock('@/pages/Workers/api/workers', () => ({
  fetchEngineWorkersList: async () => {
    order.push('engine::workers::list')
    return { workers: [...connected].map((name) => ({ name })) }
  },
}))

/** Deliver one event to every live binding of `type`. */
function emit(type: string, payload: unknown) {
  for (const binding of bindings) {
    if (binding.active && binding.type === type) {
      handlers.get(binding.function_id)?.(payload)
    }
  }
}

const live = (type: string) =>
  bindings.filter((binding) => binding.active && binding.type === type)

const calls = (fn: string) =>
  trigger.mock.calls.filter(([name]) => name === fn).length

/** Let the awaited calls behind an event settle. */
const settle = () => vi.advanceTimersByTimeAsync(0)

function resetBus() {
  trigger.mockReset()
  handlers.clear()
  bindings.length = 0
  order.length = 0
  connected.clear()
}

const META = {
  name: 'DEEPSEEK_API_KEY',
  hint: 'sk-e2e…7777',
  consumers: ['llm-router'],
}

const context = {
  consoleConfig: null,
  signal: { cancelled: false },
  report: () => undefined,
}

function storeStep(
  input: Extract<PlanStep, { kind: 'store-secret' }>['input'],
): PlanStep {
  return {
    kind: 'store-secret',
    name: 'DEEPSEEK_API_KEY',
    input,
    owner: 'DeepSeek',
    consumers: ['llm-router'],
    envFile: '.env.staging',
  }
}

describe('runStep store-secret', () => {
  beforeEach(() => {
    trigger.mockReset()
    trigger.mockImplementation(async (fn: string) =>
      fn === 'secrets::get' ? null : META,
    )
  })

  it('shares a variable as it is, without claiming it was stored', async () => {
    const result = await runStep(storeStep({ mode: 'env' }), context)
    expect(result.note).toBeUndefined()
    expect(trigger).toHaveBeenCalledWith(
      'secrets::access',
      { name: 'DEEPSEEK_API_KEY', consumers: ['llm-router'], store: 'env' },
      expect.anything(),
    )
  })

  it('says a pasted key was saved, without the key or its hint', async () => {
    const result = await runStep(
      storeStep({ mode: 'paste', value: 'sk-e2e-pasted-7777', store: 'env' }),
      context,
    )
    expect(result.note).toBe('saved')
  })

  it('says the same for the encrypted store, never the reference', async () => {
    const result = await runStep(
      storeStep({ mode: 'paste', value: 'sk-e2e-pasted-7777' }),
      context,
    )
    expect(result.note).toBe('saved')
    expect(result.note).not.toContain('secret://')
  })
})

type Snapshot = {
  status: 'running' | 'succeeded' | 'failed' | 'cancelled'
  phase: string
  completed: number
  total: number
  last_event?: { detail?: string } | null
}

describe('runStep add-workers', () => {
  let snapshot: Snapshot | Error
  let acceptedId: string | null | undefined
  let report: ReturnType<typeof vi.fn<(progress: StepProgress) => void>>
  const addStep: PlanStep = {
    kind: 'add-workers',
    workers: ['judge'],
    why: { judge: 'runs the judge' },
  }
  const operationId = () =>
    (
      trigger.mock.calls.find(([fn]) => fn === 'compose::add')?.[1] as {
        operation_id: string
      }
    )?.operation_id

  beforeEach(() => {
    vi.useFakeTimers()
    resetBus()
    snapshot = { status: 'running', phase: 'accepted', completed: 0, total: 1 }
    acceptedId = undefined
    report = vi.fn<(progress: StepProgress) => void>()
    trigger.mockImplementation(
      async (fn: string, payload: Record<string, unknown>) => {
        order.push(fn)
        if (fn === 'compose::add') {
          return {
            operation_id:
              acceptedId === undefined ? payload.operation_id : acceptedId,
          }
        }
        if (fn === 'compose::operation') {
          if (snapshot instanceof Error) throw snapshot
          return snapshot
        }
        return null
      },
    )
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const start = (signal = { cancelled: false }) =>
    runStep(addStep, { consoleConfig: null, report, signal })

  it('binds compose-operation before compose::add and follows its events', async () => {
    const done = start()
    await settle()
    const id = operationId()
    expect(id).toMatch(/^ade-setup:/)
    expect(order.indexOf('subscribe compose-operation')).toBeLessThan(
      order.indexOf('compose::add'),
    )
    expect(live('compose-operation')[0].config).toEqual({
      operation_id: id,
      terminal_only: false,
    })
    // One read covers the instant before the binding was live.
    expect(calls('compose::operation')).toBe(1)

    emit('compose-operation', {
      operation_id: id,
      phase: 'starting',
      container: 'judge',
      detail: 'pulling the image',
      current: 1,
      total: 2,
      terminal: false,
    })
    await settle()
    expect(report).toHaveBeenLastCalledWith({
      note: 'starting · judge · pulling the image',
      progress: 0.5,
    })
    // Progress is enough on its own: no read per event.
    expect(calls('compose::operation')).toBe(1)

    snapshot = {
      status: 'succeeded',
      phase: 'complete',
      completed: 1,
      total: 1,
    }
    connected.add('judge')
    emit('compose-operation', {
      operation_id: id,
      phase: 'complete',
      detail: 'all requested workers are ready',
      current: 1,
      total: 1,
      terminal: true,
    })
    await expect(done).resolves.toEqual({ note: 'judge running' })
    // The terminal event carries no status: exactly one read for it.
    expect(calls('compose::operation')).toBe(2)
    expect(bindings.every((binding) => !binding.active)).toBe(true)
    expect(handlers.size).toBe(0)
  })

  it('fails with the operation detail when the terminal read says failed', async () => {
    const done = start()
    await settle()
    snapshot = {
      status: 'failed',
      phase: 'complete',
      completed: 0,
      total: 1,
      last_event: { detail: 'one or more workers failed' },
    }
    emit('compose-operation', {
      operation_id: operationId(),
      phase: 'complete',
      detail: 'one or more workers failed',
      terminal: true,
    })
    await expect(done).rejects.toThrow('one or more workers failed')
  })

  it('reads the terminal detail when the status read fails', async () => {
    const done = start()
    await settle()
    snapshot = new Error('compose::operation unavailable')
    connected.add('judge')
    emit('compose-operation', {
      operation_id: operationId(),
      phase: 'complete',
      detail: 'all requested workers are ready',
      terminal: true,
    })
    await expect(done).resolves.toEqual({ note: 'judge running' })
  })

  it('catches up when the operation finished before the binding was live', async () => {
    snapshot = {
      status: 'succeeded',
      phase: 'complete',
      completed: 1,
      total: 1,
    }
    const compose = trigger.getMockImplementation()
    trigger.mockImplementation(
      async (fn: string, payload: Record<string, unknown>) => {
        // The worker is up by the time compose answers.
        if (fn === 'compose::add') connected.add('judge')
        return compose?.(fn, payload)
      },
    )
    await expect(start()).resolves.toEqual({ note: 'judge running' })
    expect(calls('compose::operation')).toBe(1)
  })

  it('follows the operation id an older compose chose instead of ours', async () => {
    acceptedId = 'compose:theirs'
    const done = start()
    await settle()
    expect(live('compose-operation').map((b) => b.config.operation_id)).toEqual(
      [operationId(), 'compose:theirs'],
    )
    expect(trigger).toHaveBeenCalledWith(
      'compose::operation',
      { operation_id: 'compose:theirs' },
      expect.anything(),
    )
    emit('compose-operation', {
      operation_id: operationId(),
      phase: 'ignored',
      detail: 'not this one',
      terminal: false,
    })
    emit('compose-operation', {
      operation_id: 'compose:theirs',
      phase: 'starting',
      detail: 'judge',
      terminal: false,
    })
    await settle()
    expect(report).toHaveBeenLastCalledWith({
      note: 'starting · judge',
      progress: undefined,
    })
    snapshot = {
      status: 'succeeded',
      phase: 'complete',
      completed: 1,
      total: 1,
    }
    connected.add('judge')
    emit('compose-operation', {
      operation_id: 'compose:theirs',
      phase: 'complete',
      detail: 'all requested workers are ready',
      terminal: true,
    })
    await expect(done).resolves.toEqual({ note: 'judge running' })
  })

  it('reads once after a silence and not again until another event', async () => {
    const done = start()
    await settle()
    expect(calls('compose::operation')).toBe(1)
    await vi.advanceTimersByTimeAsync(30_000)
    expect(calls('compose::operation')).toBe(2)
    await vi.advanceTimersByTimeAsync(120_000)
    expect(calls('compose::operation')).toBe(2)

    emit('compose-operation', {
      operation_id: operationId(),
      phase: 'starting',
      detail: 'still going',
      terminal: false,
    })
    await vi.advanceTimersByTimeAsync(29_000)
    expect(calls('compose::operation')).toBe(2)
    await vi.advanceTimersByTimeAsync(1_000)
    expect(calls('compose::operation')).toBe(3)

    snapshot = {
      status: 'succeeded',
      phase: 'complete',
      completed: 1,
      total: 1,
    }
    connected.add('judge')
    emit('compose-operation', {
      operation_id: operationId(),
      phase: 'complete',
      detail: 'all requested workers are ready',
      terminal: true,
    })
    await expect(done).resolves.toEqual({ note: 'judge running' })
  })

  it('waits for the worker to connect on engine::workers-available', async () => {
    acceptedId = null // compose finished in-line, no operation to follow
    const done = start()
    await settle()
    expect(report).toHaveBeenLastCalledWith({
      note: 'waiting for judge to connect',
    })
    expect(live('engine::workers-available')).toHaveLength(1)
    const lists = order.filter((entry) => entry === 'engine::workers::list')
    // Nothing more is asked while nothing happens.
    await vi.advanceTimersByTimeAsync(20_000)
    expect(order.filter((entry) => entry === 'engine::workers::list')).toEqual(
      lists,
    )
    connected.add('judge')
    emit('engine::workers-available', {
      event: 'worker_connected',
      worker_id: 'w1',
    })
    await expect(done).resolves.toEqual({ note: 'judge running' })
    expect(live('engine::workers-available')).toHaveLength(0)
  })

  it('gives up when the worker never connects', async () => {
    acceptedId = null
    const done = start()
    const failed = expect(done).rejects.toThrow('judge did not start in time')
    await vi.advanceTimersByTimeAsync(600_000)
    await failed
  })

  it('ends at once when the run is cancelled, without waiting for an event', async () => {
    const signal = { cancelled: false }
    const done = start(signal)
    await settle()
    const failed = expect(done).rejects.toThrow('cancelled')
    signal.cancelled = true
    await failed
    expect(live('compose-operation')).toHaveLength(0)
    // The token is a plain object again.
    expect(Object.getOwnPropertyDescriptor(signal, 'cancelled')).toMatchObject({
      value: true,
      writable: true,
    })
  })
})

describe('composeEventProgress', () => {
  it('names the container once and ignores a depth posing as a total', () => {
    expect(
      composeEventProgress({
        operation_id: 'op',
        phase: 'waiting',
        container: 'judge',
        detail: 'judge waits for llm-router',
        current: null,
        total: 3,
        terminal: false,
      }),
    ).toEqual({
      note: 'waiting · judge waits for llm-router',
      progress: undefined,
    })
  })
})

describe('runStep set-config', () => {
  let entries: { id: string }[]

  beforeEach(() => {
    vi.useFakeTimers()
    resetBus()
    entries = []
    trigger.mockImplementation(async (fn: string) => {
      order.push(fn)
      if (fn === 'configuration::list') return { configurations: entries }
      if (fn === 'configuration::get') return { value: {} }
      return null
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const step: PlanStep = {
    kind: 'set-config',
    configuration: 'judge',
    path: ['provider'],
    value: 'laya',
    owner: 'Laya',
  }

  it('writes straight away when the entry exists, without subscribing', async () => {
    entries = [{ id: 'judge' }]
    // The entry id is the engine's business, not a line in setup's log.
    await expect(runStep(step, context)).resolves.toEqual({})
    expect(bindings).toHaveLength(0)
  })

  it('waits for the entry on configuration events, bound without an id', async () => {
    const done = runStep(step, context)
    await settle()
    const [binding] = live('configuration')
    // No configuration_id: a per-id binding would start the entry's TTL.
    expect(binding.config).toEqual({
      event_types: ['configuration:registered', 'configuration:updated'],
    })
    expect(calls('configuration::list')).toBe(2)
    entries = [{ id: 'judge' }]
    emit('configuration', { event_type: 'registered', id: 'judge' })
    await expect(done).resolves.toEqual({})
    expect(trigger).toHaveBeenCalledWith('configuration::set', {
      id: 'judge',
      value: { provider: 'laya' },
    })
  })

  it('looks one last time at the deadline, then says it is missing', async () => {
    const done = runStep(step, context)
    const failed = expect(done).rejects.toThrow(
      'the judge configuration is not registered',
    )
    await vi.advanceTimersByTimeAsync(30_000)
    await failed
    // first look, the subscribed check, one silence check, the deadline look
    expect(calls('configuration::list')).toBe(4)
  })
})

describe('runStep wait-models', () => {
  let models: number
  let refreshCount: number
  let configured: boolean

  beforeEach(() => {
    vi.useFakeTimers()
    resetBus()
    models = 0
    refreshCount = 0
    configured = true
    trigger.mockImplementation(async (fn: string) => {
      order.push(fn)
      if (fn === 'router::models::list') {
        return { models: Array.from({ length: models }, () => ({})) }
      }
      if (fn === 'router::provider::list') {
        return {
          providers: [{ id: 'deepseek', configured, available: true }],
        }
      }
      if (fn === 'provider::deepseek::refresh_models') {
        models = refreshCount
        return { count: refreshCount }
      }
      return null
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const step: PlanStep = {
    kind: 'wait-models',
    providerId: 'deepseek',
    title: 'DeepSeek',
  }

  it('counts the models a refresh with the new key brings in', async () => {
    refreshCount = 4
    await expect(runStep(step, context)).resolves.toEqual({ note: '4 models' })
  })

  it('says the key was rejected after the second empty answer', async () => {
    const done = runStep(step, context)
    const failed = expect(done).rejects.toThrow(
      'DeepSeek returned no models for this key',
    )
    await settle()
    expect(calls('provider::deepseek::refresh_models')).toBe(1)
    // Someone else's catalog changing is not a reason to ask again.
    emit('router::models::changed', { provider: 'openai', count: 9 })
    await settle()
    expect(calls('provider::deepseek::refresh_models')).toBe(1)
    emit('router::models::changed', { provider: 'deepseek', count: 0 })
    await failed
    expect(calls('provider::deepseek::refresh_models')).toBe(2)
  })

  it('waits for the credential on router events', async () => {
    configured = false
    const done = runStep(step, context)
    await settle()
    expect(calls('provider::deepseek::refresh_models')).toBe(0)
    configured = true
    refreshCount = 2
    emit('router::provider::changed', { provider: 'deepseek', op: 'register' })
    await expect(done).resolves.toEqual({ note: '2 models' })
  })
})

describe('runStep check-judge', () => {
  let answer: unknown

  beforeEach(() => {
    vi.useFakeTimers()
    resetBus()
    trigger.mockImplementation(async (fn: string) => {
      order.push(fn)
      if (fn === 'judge::models::list') {
        if (answer instanceof Error) throw answer
        return answer
      }
      return null
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const step: PlanStep = { kind: 'check-judge', title: 'Laya', hosted: false }
  const functions = (...ids: string[]) => ({
    event: 'functions_changed',
    functions: ids.map((function_id) => ({ function_id })),
  })

  it('asks again only when new judge functions register', async () => {
    answer = { status: 'error', code: 'provider_unavailable' }
    const done = runStep(step, context)
    await settle()
    expect(calls('judge::models::list')).toBe(1)
    // The long wait lets a local provider finish loading inside the call.
    expect(trigger).toHaveBeenCalledWith(
      'judge::models::list',
      { timeout_ms: 300_000 },
      { timeoutMs: 310_000 },
    )
    emit('engine::functions-available', functions('judge::evaluate', 'x::y'))
    await settle()
    expect(calls('judge::models::list')).toBe(2)
    // The same judge functions again (another worker changed): no new ask.
    emit('engine::functions-available', functions('judge::evaluate', 'z::w'))
    await settle()
    expect(calls('judge::models::list')).toBe(2)
    answer = { models: [{ name: 'laya' }] }
    emit(
      'engine::functions-available',
      functions('judge::evaluate', 'judge-laya::models::list'),
    )
    await expect(done).resolves.toEqual({ note: 'answering' })
  })

  it('falls back to the default wait when the provider refuses the long one', async () => {
    answer = { status: 'error', code: 'invalid_request' }
    trigger.mockImplementation(
      async (fn: string, payload: Record<string, unknown>) => {
        if (fn !== 'judge::models::list') return null
        return 'timeout_ms' in payload ? answer : { models: [] }
      },
    )
    await expect(runStep(step, context)).resolves.toEqual({ note: 'answering' })
  })

  it('fails at once on a rejected hosted key', async () => {
    answer = { status: 'error', http_status: 401 }
    await expect(
      runStep({ kind: 'check-judge', title: 'Jev', hosted: true }, context),
    ).rejects.toThrow('Jev rejected this key')
  })

  it('reads the judge functions out of a registry event', () => {
    expect(
      judgeFunctionsSignature(
        functions('state::get', 'judge-laya::evaluate', 'judge::evaluate'),
      ),
    ).toBe('judge-laya::evaluate,judge::evaluate')
    expect(judgeFunctionsSignature({ event: 'other' })).toBeNull()
  })
})

describe('checkProviderKey', () => {
  let configured: boolean

  beforeEach(() => {
    vi.useFakeTimers()
    resetBus()
    configured = false
    trigger.mockImplementation(async (fn: string) => {
      if (fn === 'router::provider::list') {
        return { providers: [{ id: 'deepseek', configured, available: true }] }
      }
      if (fn === 'router::models::list') return { models: [{}, {}] }
      if (fn === 'provider::deepseek::refresh_models') return { count: 2 }
      return { models: [] }
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('looks again when the router reports the provider', async () => {
    const done = checkProviderKey('deepseek')
    await settle()
    expect(calls('router::provider::list')).toBe(2)
    configured = true
    emit('router::models::changed', { provider: 'deepseek', count: 2 })
    await expect(done).resolves.toEqual({ configured: true, models: 2 })
  })

  it('takes what is there at the deadline', async () => {
    const done = checkProviderKey('deepseek', { settleMs: 6_000 })
    await vi.advanceTimersByTimeAsync(6_000)
    await expect(done).resolves.toEqual({
      configured: false,
      models: 0,
      error: undefined,
    })
    expect(calls('router::provider::list')).toBe(3)
  })
})

describe('runStep add-workers: the compose file', () => {
  const FILE = '/repo/harness/worker-compose.yaml'
  const addStep: PlanStep = {
    kind: 'add-workers',
    workers: ['provider-claude-code'],
    why: { 'provider-claude-code': 'Claude Code' },
  }
  const answer = (listing: () => unknown) => async (fn: string) => {
    if (fn === 'compose::list') return listing()
    if (fn === 'compose::add') {
      // The worker is up by the time compose answers, with no operation to follow.
      connected.add('provider-claude-code')
      return {}
    }
    return null
  }

  beforeEach(() => {
    vi.useFakeTimers()
    resetBus()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('names the file the daemon loaded, not whatever is in its working directory', async () => {
    trigger.mockImplementation(
      answer(() => ({ projects: [{ file: FILE, namespace: 'my-project' }] })),
    )
    await expect(runStep(addStep, context)).resolves.toEqual({
      note: 'provider-claude-code running',
    })
    expect(trigger).toHaveBeenCalledWith(
      'compose::add',
      expect.objectContaining({
        file: FILE,
        workers: ['provider-claude-code'],
      }),
      expect.anything(),
    )
  })

  it('lets compose keep its default when the daemon cannot say', async () => {
    trigger.mockImplementation(
      answer(() => {
        throw new Error('function_not_found')
      }),
    )
    await expect(runStep(addStep, context)).resolves.toEqual({
      note: 'provider-claude-code running',
    })
    const payload = trigger.mock.calls.find(
      ([fn]) => fn === 'compose::add',
    )?.[1]
    expect(payload).toMatchObject({ workers: ['provider-claude-code'] })
    expect(payload).not.toHaveProperty('file')
  })
})
