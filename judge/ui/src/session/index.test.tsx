// @vitest-environment jsdom

import type { WorkerConfigurationPanelProps } from '@iii-dev/console-ui'
import { act, type ReactNode, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Engine } from './engine'
import { JudgeSessionPicker, SESSION_PROVIDER_KEY } from './index'

// The console provides these via its import map; the menu opens on mount,
// and a hidden control asks it to close the way an outside click would.
vi.mock('@iii-dev/console-ui', () => ({
  DropdownMenu: ({ children, onOpenChange }: { children: ReactNode; onOpenChange?(open: boolean): void }) => {
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only open
    useEffect(() => onOpenChange?.(true), [])
    return (
      <div>
        <button type="button" data-close-menu onClick={() => onOpenChange?.(false)} />
        {children}
      </div>
    )
  },
  DropdownMenuTrigger: ({ children, ...props }: { children: ReactNode }) => <button {...props}>{children}</button>,
  DropdownMenuContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  Button: ({ children, variant: _v, size: _s, ...props }: { children: ReactNode; variant?: string; size?: string }) => (
    <button type="button" {...props}>
      {children}
    </button>
  ),
  IconButton: ({ children, label, tooltip, tooltipSide: _s, variant: _v, ...props }: {
    children: ReactNode
    label: string
    tooltip?: ReactNode | false
    tooltipSide?: string
    variant?: string
  }) => (
    <>
      <button type="button" aria-label={label} {...props}>
        {children}
      </button>
      {tooltip ? <div role="tooltip">{tooltip}</div> : null}
    </>
  ),
  Skeleton: () => <span data-skeleton />,
  uiClasses: { spin: 'spin', motionPickerPage: 'picker-page' },
  StatusBar: ({ children }: { children: ReactNode }) => <footer>{children}</footer>,
  WorkerConfigurationDialog: ({ configurationId }: { configurationId: string | null }) =>
    configurationId ? <output data-settings-dialog={configurationId} /> : null,
}))

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

let root: Root | null = null
let host: HTMLElement | null = null
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  act(() => root?.unmount())
  host?.remove()
  root = null
  host = null
})

const MODELS: Record<string, string> = {
  'judge-typesafe': 'jev-latest',
  'judge-semif': 'qwen3.5-4b',
  'judge-decider': 'decider-4b-v2',
}

function engine(registered = ['typesafe', 'semif']) {
  const state = {
    registered: [...registered],
    models: { ...MODELS },
    addReply: { status: 'accepted' } as unknown,
    operation: { status: 'running' } as unknown,
  }
  const trigger = vi.fn(async (id: string, payload?: Record<string, unknown>) => {
    if (id === 'engine::functions::list') {
      return {
        functions: state.registered.map((p) => ({ function_id: `judge-${p}::evaluate`, worker_name: `judge-${p}` })),
      }
    }
    if (id === 'judge::configuration-id') return { id: 'judge' }
    const identity = /^(judge-[a-z0-9-]+)::configuration-id$/.exec(id)
    if (identity) return { id: identity[1] }
    if (id === 'configuration::get') {
      return payload?.id === 'judge' ? { value: { provider: 'typesafe' } } : { value: { model: state.models[String(payload?.id)] } }
    }
    if (id === 'compose::add') {
      if (state.addReply instanceof Error) throw state.addReply
      // The daemon runs it under the caller's id.
      return state.addReply &&
        typeof state.addReply === 'object' &&
        (state.addReply as { status?: string }).status === 'accepted'
        ? { ...state.addReply, operation_id: payload?.operation_id }
        : state.addReply
    }
    if (id === 'compose::operation') return state.operation
    return {}
  })
  // The bus: handlers the page registers and the bindings that route to them.
  const handlers = new Map<string, (payload: unknown) => void>()
  const bindings: { type: string; function_id: string; config: Record<string, unknown> }[] = []
  const on = vi.fn((id: string, handler: (payload: unknown) => void) => {
    handlers.set(id, handler)
    return () => handlers.delete(id)
  })
  const registerTrigger = vi.fn((input: { type: string; function_id: string; config: Record<string, unknown> }) => {
    bindings.push(input)
    return () => {
      const at = bindings.indexOf(input)
      if (at >= 0) bindings.splice(at, 1)
    }
  })
  /** Deliver `payload` to every binding of `type` whose config `match` accepts. */
  const emit = (type: string, payload: unknown, match: (config: Record<string, unknown>) => boolean = () => true) => {
    for (const binding of [...bindings]) {
      if (binding.type !== type || !match(binding.config)) continue
      handlers.get(binding.function_id.replace(/::console-test$/, ''))?.(payload)
    }
  }
  return Object.assign({ trigger, on, registerTrigger, browserId: 'console-test' }, { state, bindings, handlers, emit })
}

/** The operation id the page bound its `compose-operation` progress to. */
function boundOperation(iii: ReturnType<typeof engine>): string {
  const binding = iii.bindings.find((entry) => entry.type === 'compose-operation')
  if (!binding) throw new Error('no compose-operation binding')
  return String(binding.config.operation_id)
}

/** Compose ends the operation: its terminal event reaches the page. */
function endOperation(iii: ReturnType<typeof engine>, operationId: string) {
  iii.emit(
    'compose-operation',
    { operation_id: operationId, sequence: 9, phase: 'complete', detail: '', terminal: true },
    (config) => config.operation_id === operationId,
  )
}

/** Past the live list's debounce. */
const settle = (ms = 350) =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms))
  })

/** The registry's `tag=evaluation` page: judges plus unrelated workers. */
function registry(ok = true) {
  const page = {
    workers: [
      { name: 'judge-typesafe', version: '0.2.1', description: 'TypeSafe evaluation' },
      { name: 'judge', version: '0.2.1', description: 'Provider-neutral evaluation hub' },
      { name: 'judge-decider', version: '0.1.0', description: 'decider provider for the judge hub' },
      { name: 'judge-semif', version: '0.1.0', description: 'SemIf provider for the judge hub' },
      { name: 'eval', version: '0.2.14', description: 'benchmarks' },
    ],
    pagination: { has_more: false },
  }
  return vi.fn(async (url: string) => {
    expect(url).toBe('https://api.workers.iii.dev/w?tag=evaluation')
    return ok ? new Response(JSON.stringify(page), { status: 200 }) : new Response('down', { status: 503 })
  })
}

/** The Console's inline editor, reduced to what the picker relies on. */
function FakePanel({ configurationId, onDirtyChange, onSaved }: WorkerConfigurationPanelProps) {
  return (
    <section data-panel={configurationId}>
      <button type="button" onClick={() => onDirtyChange?.(true)}>
        edit
      </button>
      <button type="button" onClick={() => onSaved?.({})}>
        save
      </button>
    </section>
  )
}

async function mount(
  metadata: Record<string, unknown>,
  {
    iii = engine(),
    setMetadata = vi.fn<(patch: Record<string, unknown>) => void>(),
    panel = FakePanel as typeof FakePanel | null,
  }: { iii?: ReturnType<typeof engine>; setMetadata?: ReturnType<typeof vi.fn<(patch: Record<string, unknown>) => void>>; panel?: typeof FakePanel | null } = {},
) {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root!.render(
      <JudgeSessionPicker
        iii={iii as unknown as Engine}
        configurationPanel={panel ?? undefined}
        sessionId="s1"
        isStreaming={false}
        metadata={metadata}
        setMetadata={setMetadata}
      />,
    )
  })
  // Let the listing (functions, then each judge's settings) settle.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  const view = host
  const option = (id: string) => view.querySelector<HTMLButtonElement>(`[data-judge-option="${id}"]`)
  const options = () => [...view.querySelectorAll<HTMLElement>('[data-judge-option]')].map((row) => row.dataset.judgeOption)
  const labels = () =>
    [...view.querySelectorAll('[data-judge-option] .judge-ui-session-option-label')].map((label) => label.textContent)
  // On the page on screen first: every page stays mounted, the others inert.
  const button = (name: string) => {
    const scopes = [view.querySelector('[data-page][data-active="true"]'), view]
    const found = scopes
      .flatMap((scope) => [...(scope?.querySelectorAll<HTMLButtonElement>('button') ?? [])])
      .find((candidate) => (candidate.getAttribute('aria-label') ?? candidate.textContent?.trim()) === name)
    if (!found) throw new Error(`no button ${name}: ${view.textContent}`)
    return found
  }
  const activePage = () => view.querySelector<HTMLElement>('[data-page][data-active="true"]')?.dataset.page
  return { view, iii, setMetadata, option, options, labels, button, activePage }
}

async function type(input: HTMLInputElement, text: string) {
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!
    setValue.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

async function press(input: HTMLInputElement, key: string) {
  await act(async () => {
    input.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
  })
}

describe('per-session judge provider', () => {
  it("lists Default and each running judge with its model, and writes only this session's metadata key", async () => {
    const { view, iii, setMetadata, option, options, labels } = await mount({
      [SESSION_PROVIDER_KEY]: 'typesafe',
      model: 'm',
    })
    expect(view.querySelector('.judge-ui-session-trigger')?.textContent).toBe('judge · typesafe')
    expect(options()).toEqual(['default', 'semif', 'typesafe'])
    expect(labels()).toEqual(['Default', 'qwen3.5-4b', 'jev-latest'])
    expect(option('default')?.textContent).toContain('typesafe · jev-latest')
    expect(option('typesafe')?.getAttribute('aria-pressed')).toBe('true')
    // A judge's model comes from its settings, read raw: no `${VAR}` expands into the page.
    expect(iii.trigger).toHaveBeenCalledWith(
      'configuration::get',
      { id: 'judge-semif', raw: true },
      { timeoutMs: 5_000 },
    )
    expect(iii.trigger.mock.calls.some(([id]) => id === 'judge::models::list')).toBe(false)
    await act(async () => option('semif')!.click())
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: 'semif' })
    // Picking a provider starts its model load before the session's next turn.
    expect(iii.trigger).toHaveBeenCalledWith(
      'judge::models::list',
      { provider: 'semif', timeout_ms: 1_000 },
      { timeoutMs: 10_000 },
    )
    const calls = iii.trigger.mock.calls.length
    await act(async () => option('default')!.click())
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: undefined })
    expect(iii.trigger.mock.calls.length).toBe(calls)
  })

  it('keeps a stored provider that is not running selectable', async () => {
    const { option, view } = await mount({ [SESSION_PROVIDER_KEY]: 'laya' })
    expect(option('laya')?.textContent).toContain('Not running')
    expect(option('laya')?.getAttribute('aria-pressed')).toBe('true')
    // Nothing answers for its settings, so there is nothing to configure.
    expect(view.querySelector('button[aria-label="Configure laya"]')).toBeNull()
  })

  it('filters the judges, moves with the arrows and picks on Enter', async () => {
    const { view, setMetadata, options } = await mount({})
    const filter = view.querySelector<HTMLInputElement>('input[aria-label="Filter judges"]')!
    // The highlight starts on the session's choice (Default) and walks down.
    expect(view.querySelector('[data-highlighted]')?.getAttribute('data-judge-option')).toBe('default')
    await press(filter, 'ArrowDown')
    expect(view.querySelector('[data-highlighted]')?.getAttribute('data-judge-option')).toBe('semif')
    expect(filter.getAttribute('aria-activedescendant')).toBe(view.querySelector('[data-highlighted]')?.id)
    await press(filter, 'Enter')
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: 'semif' })
    // Default matches what it resolves to.
    await type(filter, 'jev')
    expect(options()).toEqual(['default', 'typesafe'])
    await press(filter, 'ArrowDown')
    await press(filter, 'Enter')
    expect(setMetadata).toHaveBeenLastCalledWith({ [SESSION_PROVIDER_KEY]: 'typesafe' })
    await type(filter, 'nothing')
    expect(options()).toEqual([])
    expect(view.textContent).toContain('No judge matches “nothing”.')
  })

  it("configures a judge inside the picker and comes back to the list", async () => {
    const { view, button, activePage } = await mount({})
    expect(activePage()).toBe('judges')
    await act(async () => button('Configure typesafe').click())
    expect(activePage()).toBe('configure')
    expect(view.querySelector('[data-page="configure"] h2')?.textContent).toBe('typesafe')
    expect(view.querySelector('[data-panel]')?.getAttribute('data-panel')).toBe('judge-typesafe')
    await act(async () => button('Back to judges').click())
    expect(activePage()).toBe('judges')
    await act(async () => button('Configure judge settings').click())
    expect(view.querySelector('[data-page="configure"] h2')?.textContent).toBe('Judge settings')
    expect(view.querySelector('[data-panel]')?.getAttribute('data-panel')).toBe('judge')
  })

  it('asks before leaving settings with unsaved edits', async () => {
    const { view, button, activePage } = await mount({})
    await act(async () => button('Configure typesafe').click())
    await act(async () => button('edit').click())
    await act(async () => button('Back to judges').click())
    expect(activePage()).toBe('configure')
    expect(view.querySelector('[role="alert"]')?.textContent).toContain('Discard the changes you have not saved?')
    await act(async () => button('Keep editing').click())
    expect(view.querySelector('[role="alert"]')).toBeNull()
    // Closing the menu asks too, and keeps it open until answered.
    await act(async () => view.querySelector<HTMLElement>('[data-close-menu]')!.click())
    expect(activePage()).toBe('configure')
    await act(async () => button('Discard').click())
    expect(activePage()).toBe('judges')
  })

  it('reads the judges again after a save, so a new model shows', async () => {
    const iii = engine()
    const { button, labels } = await mount({}, { iii })
    await act(async () => button('Configure semif').click())
    iii.state.models['judge-semif'] = 'qwen3.5-9b'
    await act(async () => button('save').click())
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(labels()).toContain('qwen3.5-9b')
  })

  it('opens the settings in Settings on a Console without the inline editor', async () => {
    const { view, button, activePage } = await mount({}, { panel: null })
    await act(async () => button('Configure judge settings').click())
    expect(activePage()).toBe('judges')
    expect(view.querySelector('[data-settings-dialog]')?.getAttribute('data-settings-dialog')).toBe('judge')
  })

  it('adds a registry judge that is not installed, follows it until it registers, then offers its settings', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    // Accepted, then still running when the judge registers.
    iii.state.addReply = { status: 'accepted', operation_id: 'op-2', requested: 1 }
    const { view, button, options, activePage } = await mount({}, { iii })
    await act(async () => button('Add a judge').click())
    await act(async () => {})
    expect(activePage()).toBe('add')
    // Installed judges and the hub itself stay off the page.
    const rows = () => [...view.querySelectorAll('.judge-ui-session-add-title [data-label]')].map((row) => row.textContent)
    expect(rows()).toEqual(['decider'])
    await act(async () => button('Add decider').click())
    const operationId = boundOperation(iii)
    expect(operationId).toMatch(/^compose:[0-9a-f-]{36}$/)
    // Bound terminal-only, before the add, under the id the add then names.
    expect(iii.registerTrigger).toHaveBeenCalledWith({
      type: 'compose-operation',
      function_id: expect.stringMatching(/^iii::judge-ui::compose-operation::\d+::console-test$/),
      config: { operation_id: operationId, terminal_only: true },
    })
    const bound = iii.registerTrigger.mock.invocationCallOrder.at(-1)!
    const added = iii.trigger.mock.calls.findIndex(([id]) => id === 'compose::add')
    expect(iii.trigger.mock.invocationCallOrder[added]).toBeGreaterThan(bound)
    expect(iii.trigger).toHaveBeenCalledWith(
      'compose::add',
      { workers: ['judge-decider'], operation_id: operationId },
      { timeoutMs: 600_000 },
    )
    expect(view.textContent).toContain('Adding…')
    // The judge registering is an engine event, not something polled for.
    iii.state.registered.push('decider')
    await act(async () => iii.emit('engine::functions-available', { event: 'functions_changed', functions: [] }))
    await settle()
    await act(async () => button('Configure').click())
    expect(activePage()).toBe('configure')
    expect(view.querySelector('[data-panel]')?.getAttribute('data-panel')).toBe('judge-decider')
    await act(async () => button('Back to judges').click())
    expect(options()).toEqual(['default', 'decider', 'semif', 'typesafe'])
  }, 10_000)

  it('shows why an add failed and offers a retry', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    iii.state.addReply = { status: 'failed', error: { message: 'worker judge-decider not found' } }
    const { view, button } = await mount({}, { iii })
    await act(async () => button('Add a judge').click())
    await act(async () => {})
    await act(async () => button('Add decider').click())
    await act(async () => {})
    expect(view.querySelector('[role="alert"]')?.textContent).toBe('worker judge-decider not found')
    expect(view.querySelector('button[aria-label="Retry adding decider"]')).not.toBeNull()
  })

  it('shows why the compose operation failed after it was accepted', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    const reason = "Worker 'judge-decider' does not support platform 'x86_64-unknown-linux-musl'."
    // The daemon's detail, verbatim: the reason sits between resolver noise
    // and advice for the publisher.
    const detail = `container 'judge-decider': no version of 'judge-decider' satisfies '*'. ${reason} Publish a 'judge-decider' binary for 'x86_64-unknown-linux-musl' or install on a supported platform. (available: x86_64-unknown-linux-gnu)`
    iii.state.addReply = { status: 'accepted', requested: 1 }
    const { view, button } = await mount({}, { iii })
    await act(async () => button('Add a judge').click())
    await act(async () => {})
    await act(async () => button('Add decider').click())
    await act(async () => {})
    const operationId = boundOperation(iii)
    // The one catch-up read finds it still running: nothing more is read
    // until compose says it ended.
    expect(iii.trigger).toHaveBeenCalledWith('compose::operation', { operation_id: operationId }, { timeoutMs: 5_000 })
    const reads = () => iii.trigger.mock.calls.filter(([id]) => id === 'compose::operation').length
    expect(reads()).toBe(1)
    expect(view.textContent).toContain('Adding…')
    iii.state.operation = { status: 'failed', last_event: { terminal: true, detail } }
    await act(async () => endOperation(iii, operationId))
    await act(async () => {})
    expect(reads()).toBe(2)
    expect(view.querySelector('[role="alert"]')?.textContent).toBe(reason)
    expect(view.querySelector('button[aria-label="Retry adding decider"]')).not.toBeNull()
    // Settled: the binding is gone.
    expect(iii.bindings.some((entry) => entry.type === 'compose-operation')).toBe(false)
  })

  it('settles from the catch-up read an operation that ended before its binding landed', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    iii.state.addReply = { status: 'accepted', requested: 1 }
    iii.state.operation = { status: 'cancelled', last_event: { terminal: true, detail: 'operation cancelled' } }
    const { view, button } = await mount({}, { iii })
    await act(async () => button('Add a judge').click())
    await act(async () => {})
    await act(async () => button('Add decider').click())
    await act(async () => {})
    expect(view.querySelector('[role="alert"]')?.textContent).toBe('The add was cancelled.')
    expect(iii.bindings.some((entry) => entry.type === 'compose-operation')).toBe(false)
  })

  it('reports an add that compose calls done but whose judge never starts', async () => {
    vi.stubGlobal('fetch', registry())
    const iii = engine()
    // A worker that is not required and failed to start, or one declared and
    // stopped, still ends the operation as succeeded.
    iii.state.addReply = { status: 'accepted', requested: 1 }
    const { view, button } = await mount({}, { iii })
    vi.useFakeTimers()
    await act(async () => button('Add a judge').click())
    await act(async () => {})
    await act(async () => button('Add decider').click())
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0)
    })
    const operationId = boundOperation(iii)
    iii.state.operation = {
      status: 'succeeded',
      last_event: { terminal: true, detail: 'all requested workers are ready' },
    }
    await act(async () => endOperation(iii, operationId))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_100)
    })
    expect(view.textContent).toContain('Adding…')
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_100)
    })
    expect(view.querySelector('[role="alert"]')?.textContent).toBe(
      'judge-decider was added but has not started; check its logs in Settings → Workers.',
    )
    // It starts late: the engine says so, and the open picker settles it.
    iii.state.registered.push('decider')
    await act(async () => iii.emit('engine::functions-available', { event: 'functions_changed', functions: [] }))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400)
    })
    expect(view.querySelector('[data-page="add"] .judge-ui-session-add-row button')?.textContent).toBe('Configure')
  })

  it('never reads the judges on a timer, only on engine changes while the menu is open', async () => {
    const iii = engine()
    const { view, labels } = await mount({}, { iii })
    vi.useFakeTimers()
    const lists = () => iii.trigger.mock.calls.filter(([id]) => id === 'engine::functions::list').length
    const before = lists()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000)
    })
    expect(lists()).toBe(before)
    // The configuration binding names no id; another worker's entry is not news.
    expect(iii.bindings.find((entry) => entry.type === 'configuration')?.config).toEqual({})
    await act(async () => iii.emit('configuration', { type: 'configuration', event_type: 'updated', id: 'llm-router' }))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400)
    })
    expect(lists()).toBe(before)
    // A judge's settings changing is: a burst reads once.
    iii.state.models['judge-semif'] = 'qwen3.5-9b'
    await act(async () => {
      iii.emit('configuration', { type: 'configuration', event_type: 'updated', id: 'judge-semif' })
      iii.emit('configuration', { type: 'configuration', event_type: 'updated', id: 'judge-semif' })
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400)
    })
    expect(lists()).toBe(before + 1)
    expect(labels()).toContain('qwen3.5-9b')
    // Closing the menu unbinds both.
    await act(async () => view.querySelector<HTMLElement>('[data-close-menu]')!.click())
    expect(iii.bindings.filter((entry) => entry.type !== 'compose-operation')).toEqual([])
  })

  it('says so when the registry is unreachable', async () => {
    vi.stubGlobal('fetch', registry(false))
    const { view, button } = await mount({})
    await act(async () => button('Add a judge').click())
    await act(async () => {})
    expect(view.textContent).toContain('The workers registry is unreachable right now.')
  })

  it('explains the judge and where this session uses it beside the picker', async () => {
    const { view } = await mount({})
    expect(view.querySelector('button[aria-label="What is the judge?"]')).not.toBeNull()
    const help = [...view.querySelectorAll('[role="tooltip"]')].map((tip) => tip.textContent).join(' ')
    expect(help).toContain('A fast evaluation model for typed questions')
    expect(help).toContain('ranks functions and skills when the agent searches the catalog')
    expect(help).toContain('checks a function call’s arguments')
    expect(help).toContain('browser::run')
  })
})
